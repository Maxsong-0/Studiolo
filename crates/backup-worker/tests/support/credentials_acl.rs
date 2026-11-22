//! 两种放行形态的两侧：169 上实测的原始字节是正例，每改一项都要被拒。字节里只有权限位和账号号码。
use super::*;

/// 169（systemd 259）上 worker 凭据目录的访问 ACL 原样：属主 r-x、指名用户 995 r-x、属组 ---、掩码 r-x、其他人 ---。
const DIRECTORY: &str =
    "0200000001000500ffffffff02000500e303000004000000ffffffff10000500ffffffff20000000ffffffff";
/// 同一台机器上人侧凭据文件的访问 ACL 原样：属主 r--、指名用户 1000 r--、属组 ---、掩码 r--、其他人 ---。
const FILE: &str =
    "0200000001000400ffffffff02000400e803000004000000ffffffff10000400ffffffff20000000ffffffff";
const WORKER: u32 = 995;
const OWNER: u32 = 1000;

fn bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
        .collect()
}
/// 把第 `entry` 条（从 0 数）的类别、权限、编号换掉。
fn patched(hex: &str, entry: usize, tag: u16, permission: u16, id: u32) -> Vec<u8> {
    let mut raw = bytes(hex);
    let at = 4 + entry * 8;
    raw[at..at + 2].copy_from_slice(&tag.to_le_bytes());
    raw[at + 2..at + 4].copy_from_slice(&permission.to_le_bytes());
    raw[at + 4..at + 8].copy_from_slice(&id.to_le_bytes());
    raw
}

#[test]
fn the_shapes_systemd_actually_hands_out_are_accepted() {
    // tmpfs 上的形态：属 root，ACL 恰好只多放行服务账号。
    assert!(exclusive(
        Kind::Directory,
        0,
        0o550,
        Some(&bytes(DIRECTORY)),
        WORKER
    ));
    assert!(exclusive(Kind::File, 0, 0o440, Some(&bytes(FILE)), OWNER));
    // ramfs 上的形态：属服务账号，只有属主位，没有 ACL。
    assert!(exclusive(Kind::Directory, WORKER, 0o500, None, WORKER));
    assert!(exclusive(Kind::File, WORKER, 0o400, None, WORKER));
}

#[test]
fn ownership_and_mode_must_match_the_form_exactly() {
    let (directory, file) = (bytes(DIRECTORY), bytes(FILE));
    // 有 ACL 的形态必须属 root，权限位必须正好是属主位加掩码位。
    for (owner, mode) in [
        (WORKER, 0o550),
        (1, 0o550),
        (0, 0o500),
        (0, 0o555),
        (0, 0o750),
        (0, 0o1550),
        (0, 0o4550),
    ] {
        assert!(
            !exclusive(Kind::Directory, owner, mode, Some(&directory), WORKER),
            "{owner} {mode:o}"
        );
    }
    for (owner, mode) in [
        (OWNER, 0o440),
        (0, 0o400),
        (0, 0o444),
        (0, 0o640),
        (0, 0o550),
    ] {
        assert!(
            !exclusive(Kind::File, owner, mode, Some(&file), OWNER),
            "{owner} {mode:o}"
        );
    }
    // 没有 ACL 的形态必须属服务账号自己，只有属主位。
    for (owner, mode) in [
        (0, 0o500),
        (WORKER + 1, 0o500),
        (WORKER, 0o550),
        (WORKER, 0o700),
        (WORKER, 0o505),
        (WORKER, 0o1500),
    ] {
        assert!(
            !exclusive(Kind::Directory, owner, mode, None, WORKER),
            "{owner} {mode:o}"
        );
    }
    for (owner, mode) in [
        (0, 0o400),
        (WORKER, 0o440),
        (WORKER, 0o600),
        (WORKER, 0o404),
    ] {
        assert!(
            !exclusive(Kind::File, owner, mode, None, WORKER),
            "{owner} {mode:o}"
        );
    }
    // 目录的 ACL 不能拿来放行文件，反过来也不行。
    assert!(!exclusive(Kind::File, 0, 0o440, Some(&directory), WORKER));
    assert!(!exclusive(Kind::Directory, 0, 0o550, Some(&file), OWNER));
}

#[test]
fn any_other_grant_in_the_acl_is_refused() {
    let accepted = |raw: &[u8]| exclusive(Kind::Directory, 0, 0o550, Some(raw), WORKER);
    assert!(accepted(&bytes(DIRECTORY)));
    for (label, raw) in [
        (
            "指名用户不是 worker",
            patched(DIRECTORY, 1, USER, 5, WORKER + 1),
        ),
        ("指名用户是 root", patched(DIRECTORY, 1, USER, 5, 0)),
        ("指名用户多了写权限", patched(DIRECTORY, 1, USER, 7, WORKER)),
        (
            "指名用户少了进入权限",
            patched(DIRECTORY, 1, USER, 4, WORKER),
        ),
        (
            "指名用户换成了指名组",
            patched(DIRECTORY, 1, 0x08, 5, WORKER),
        ),
        (
            "属主多了写权限",
            patched(DIRECTORY, 0, USER_OBJ, 7, u32::MAX),
        ),
        ("属组有权限", patched(DIRECTORY, 2, GROUP_OBJ, 5, u32::MAX)),
        ("掩码更宽", patched(DIRECTORY, 3, MASK, 7, u32::MAX)),
        ("掩码更窄", patched(DIRECTORY, 3, MASK, 4, u32::MAX)),
        ("其他人有权限", patched(DIRECTORY, 4, OTHER, 4, u32::MAX)),
        ("条目顺序不对", {
            let mut raw = bytes(DIRECTORY);
            raw[4..20].rotate_left(8);
            raw
        }),
        ("版本不认识", {
            let mut raw = bytes(DIRECTORY);
            raw[0] = 3;
            raw
        }),
        ("多一条指名用户", {
            let mut raw = bytes(DIRECTORY);
            let extra = raw[12..20].to_vec();
            raw.splice(20..20, extra);
            raw
        }),
        ("少一条（没有指名用户）", {
            let mut raw = bytes(DIRECTORY);
            raw.drain(12..20);
            raw
        }),
        ("截断", bytes(DIRECTORY)[..43].to_vec()),
        ("只有版本头", bytes(DIRECTORY)[..4].to_vec()),
        ("空", Vec::new()),
    ] {
        assert!(!accepted(&raw), "{label}");
    }
    // 其余类别的编号字段内核不看：不管填什么都不影响结论。
    assert!(accepted(&patched(DIRECTORY, 0, USER_OBJ, 5, 0)));
}
