//! 凭据目录和文件「除了 root 只有 worker 账号读得到」的两种形态；纯函数，不碰文件系统。
//!
//! systemd 把凭据交给服务有两种做法（BUG-043，2026-11-22 在 169 的 systemd 259 上逐级实测）：
//! - 文件系统支持 ACL（tmpfs）：目录和文件仍属 root，ACL 里加一条指名用户放行服务账号。有 ACL 时权限位的
//!   组位显示的是掩码，所以目录看上去是 0550、文件是 0440；
//! - 不支持 ACL（老内核上的 ramfs）：把属主改成服务账号，目录 0500、文件 0400，没有 ACL。
//!
//! 两种都只让 root 和服务账号读。这里逐项核对到「恰好是这两种之一」，多一条少一条都不认。

/// `system.posix_acl_access` 的格式版本（内核 `POSIX_ACL_XATTR_VERSION`）。
const VERSION: u32 = 2;
/// 条目类别（内核 `ACL_USER_OBJ` 等）。
const USER_OBJ: u16 = 0x01;
const USER: u16 = 0x02;
const GROUP_OBJ: u16 = 0x04;
const MASK: u16 = 0x10;
const OTHER: u16 = 0x20;
/// 版本头 4 字节，加属主、指名用户、属组、掩码、其他人五条各 8 字节。读 ACL 的缓冲按它留一个字节的余量，
/// 更长的直接判不认。
pub(super) const EXACT_LEN: usize = 4 + 5 * 8;

/// 核的是目录还是文件：目录要能进（读和进入），文件只要能读。
#[derive(Clone, Copy)]
pub(super) enum Kind {
    Directory,
    File,
}
impl Kind {
    fn permission(self) -> u16 {
        match self {
            Self::Directory => 0o5,
            Self::File => 0o4,
        }
    }
}

/// 属主、权限位（含特殊位的低 12 位）和访问 ACL 的原始字节（没有 ACL 传 `None`）合起来，
/// 是不是「只有 root 和 `uid` 读得到」的两种形态之一。
pub(super) fn exclusive(kind: Kind, owner: u32, mode: u32, acl: Option<&[u8]>, uid: u32) -> bool {
    let permission = kind.permission();
    let owner_only = u32::from(permission) << 6;
    match acl {
        None => owner == uid && mode == owner_only,
        Some(raw) => {
            owner == 0
                && mode == owner_only | u32::from(permission) << 3
                && grants_only(raw, uid, permission)
        }
    }
}

/// 这份 ACL 是不是恰好五条：属主和指名用户 `uid` 各有 `permission`，掩码等于 `permission`，
/// 属组和其他人什么权限都没有。别的指名用户、指名组、条目顺序不对、长度不对都不算。
fn grants_only(raw: &[u8], uid: u32, permission: u16) -> bool {
    let Some((version, entries)) = raw.split_first_chunk::<4>() else {
        return false;
    };
    if raw.len() != EXACT_LEN || u32::from_le_bytes(*version) != VERSION {
        return false;
    }
    // 只有指名用户那一条的编号有意义；其余类别的编号字段内核不看，这里也不看。
    let expected = [
        (USER_OBJ, permission, None),
        (USER, permission, Some(uid)),
        (GROUP_OBJ, 0, None),
        (MASK, permission, None),
        (OTHER, 0, None),
    ];
    entries
        .chunks_exact(8)
        .zip(expected)
        .all(|(entry, (tag, granted, id))| {
            u16::from_le_bytes([entry[0], entry[1]]) == tag
                && u16::from_le_bytes([entry[2], entry[3]]) == granted
                && id.is_none_or(|id| {
                    u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]) == id
                })
        })
}

#[cfg(test)]
#[path = "../../tests/support/credentials_acl.rs"]
mod tests;
