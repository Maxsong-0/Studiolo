//! 挂载行判定的两侧：169 上实测的那几行是正例和对照，每改一项都要被拒。
use super::*;

/// 169（systemd 259、内核 7.0）上 `/proc/self/mountinfo` 的原样摘录：根、`/run`、两个服务的凭据挂载。
const HOST: &str = "\
48 1 252:0 / / rw,relatime shared:1 - ext4 /dev/mapper/ubuntu--vg-ubuntu--lv rw
34 48 0:29 / /run rw,nosuid,nodev shared:13 - tmpfs tmpfs rw,size=4815696k,nr_inodes=819200,mode=755,inode64
63 34 0:52 / /run/credentials/studiolo-human.service rw,nosuid,nodev,noexec,relatime,nosymfollow shared:165 - tmpfs none ro,size=1024k,nr_inodes=1024,mode=700,inode64,noswap
155 34 0:57 / /run/credentials/studiolo-backup-worker.service rw,nosuid,nodev,noexec,relatime,nosymfollow shared:514 - tmpfs none ro,size=1024k,nr_inodes=1024,mode=700,inode64,noswap
";
const WORKER: u64 = 155;

#[test]
fn the_credential_mount_systemd_actually_makes_is_accepted() {
    assert!(tmpfs_readonly_noswap(HOST, WORKER));
    assert!(tmpfs_readonly_noswap(HOST, 63));
}

#[test]
fn every_other_kind_of_mount_is_refused() {
    // 同一份 mountinfo 里别的挂载：普通磁盘、会换出的 tmpfs、不存在的挂载号。
    for id in [48, 34, 0, 156, u64::MAX] {
        assert!(!tmpfs_readonly_noswap(HOST, id), "{id}");
    }
    let line = HOST.lines().last().unwrap();
    for (label, changed) in [
        ("没有 noswap（会换出到硬盘）", line.replace(",noswap", "")),
        (
            "选项名只是以 noswap 开头",
            line.replace(",noswap", ",noswapx"),
        ),
        ("超级块可写", line.replace(" none ro,", " none rw,")),
        (
            "只有挂载点一层只读、超级块可写",
            line.replace(" none ro,", " none rw,")
                .replace(" rw,nosuid", " ro,nosuid"),
        ),
        ("不是 tmpfs", line.replace("- tmpfs none", "- ext4 none")),
        (
            "类型名只是以 tmpfs 开头",
            line.replace("- tmpfs none", "- tmpfsx none"),
        ),
        ("没有分隔符", line.replace(" - ", " ")),
        ("超级块选项后面多一个字段", format!("{line} extra")),
        (
            "缺超级块选项",
            line.replace(
                " none ro,size=1024k,nr_inodes=1024,mode=700,inode64,noswap",
                " none",
            ),
        ),
        ("挂载号不是数字", line.replacen("155", "x155", 1)),
    ] {
        assert!(!tmpfs_readonly_noswap(&changed, WORKER), "{label}");
    }
    // 同一个挂载号出现两行：不知道该信哪一行，拒绝。
    assert!(!tmpfs_readonly_noswap(&format!("{HOST}{line}\n"), WORKER));
    assert!(!tmpfs_readonly_noswap("", WORKER));
}
