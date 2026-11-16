//! 开库清扫：把原子写崩溃留下的临时文件挪进 Broker 自己的隔离区（T-207，接 T-204 复审）。
//!
//! 原子写（`fs::rooted`）先写 `.<原名>.studiolo-tmp-<进程号>-<序号>` 再改名，进程死在两步之间就留下它。
//! 它可能是任何文件的半截或全文，备份归不了类，导出扫描见到就整批拒绝；重启后进程号碰巧相同时，
//! 新的原子写还会撞上它的名字。所以开库拿到单写者租约之后、任何写入之前清扫一次：
//!
//! - 只看原子写会写的两处：Vault 根的 `.workbench/` 和整个写者数据根。用户笔记目录不碰：
//!   Broker 不在那里起临时文件，那里名字像残留的文件照旧挡备份，由主人处理（失败关闭）。
//! - 只挪名字逐段对得上、普通文件、只有一个硬链接、属于本用户、和根在同一个设备上的；
//!   目录、链接、差一点的名字都原地不动。
//! - 挪进同一个根里的隔离区 `residue-quarantine/<批次>/<原相对路径>`（Vault 侧从 `.workbench/` 算起），
//!   改名不覆盖，挪完落盘两边目录。残留对应的正式文件不读不改，未收口的写入照现有规则处理。
//! - 隔离区不在配对布局里，不参与绑定核对，封存导出跳过它（`checkpoint::tree`）；什么时候删由主人决定。
//! - 有预算：节点、深度、时间到了就停，没看到的原地不动，下次开库再挪。清扫自己出错不让开库失败。

use crate::{FsVaultBroker, fs::is_exact_temporary_name, writer::directory};
use cap_std::fs::{Dir, MetadataExt as _};
use rustix::{
    fs::{Mode, mkdirat},
    io::Errno,
};
use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// 隔离区目录名：Vault 根里在 `.workbench/` 下面，写者数据根里在顶层。
const QUARANTINE: &str = "residue-quarantine";
/// 封存跳过的 Vault 侧隔离区，逐段写。
pub(crate) const VAULT_QUARANTINE: &[&str] = &[".workbench", QUARANTINE];
/// 封存跳过的数据侧隔离区。
pub(crate) const DATA_QUARANTINE: &[&str] = &[QUARANTINE];
/// 两处合起来最多看这么多个目录项，和封存导出的上限一样。
const MAX_NODES: u64 = 1_000_000;
const MAX_DEPTH: usize = 64;
const TIME_BUDGET: Duration = Duration::from_secs(60);
/// 批次名里的进程内序号；名字被占了就换下一个。
static BATCH_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// 开库时清扫一次。只在拿到单写者租约之后、任何写入之前调用；清扫出错不让开库失败。
pub(crate) fn sweep(broker: &FsVaultBroker) {
    sweep_within(broker, Instant::now() + TIME_BUDGET, MAX_NODES);
}

/// 带预算的清扫：两处共用一份节点和时间预算，用完就停。
pub(crate) fn sweep_within(broker: &FsVaultBroker, deadline: Instant, max_nodes: u64) {
    // 根已经不是绑定的那两个就不动任何东西，后面的开库检查会照常报错。
    if broker.writer.check(&broker.root, &broker.data).is_err() {
        return;
    }
    let mut walk = Walk {
        deadline,
        remaining: max_nodes,
        uid: rustix::process::geteuid().as_raw(),
    };
    for base in [
        directory(&broker.root, ".workbench"),
        directory(&broker.data, "."),
    ] {
        let Ok(base) = base else {
            continue;
        };
        if walk.site(&base).is_err() {
            return;
        }
    }
}

/// 预算用完：停下，剩下的原地不动。
struct OutOfBudget;

struct Walk {
    deadline: Instant,
    remaining: u64,
    uid: u32,
}

/// 一处根的清扫现场：本次的隔离批次用到才建，建不成这一处就不再挪。
struct Site<'a> {
    base: &'a Dir,
    device: u64,
    batch: Batch,
}

enum Batch {
    Untried,
    Ready(Dir),
    Failed,
}

impl Walk {
    fn site(&mut self, base: &Dir) -> Result<(), OutOfBudget> {
        let Ok(metadata) = base.dir_metadata() else {
            return Ok(());
        };
        let mut site = Site {
            base,
            device: metadata.dev(),
            batch: Batch::Untried,
        };
        self.visit(&mut site, base, Path::new(""), 0)
    }

    fn visit(
        &mut self,
        site: &mut Site<'_>,
        dir: &Dir,
        relative: &Path,
        depth: usize,
    ) -> Result<(), OutOfBudget> {
        if depth > MAX_DEPTH {
            return Ok(());
        }
        // 先列全这一层再动，不在枚举中途改目录。
        let Some(entries) = self.list(dir)? else {
            return Ok(());
        };
        for (name, is_directory) in entries {
            self.in_time()?;
            if depth == 0 && name == QUARANTINE {
                continue;
            }
            if is_directory {
                // 逐段 nofollow 打开；别的设备上的目录（挂载点）不进去。
                if let Ok(child) = directory(dir, &name)
                    && child
                        .dir_metadata()
                        .is_ok_and(|metadata| metadata.dev() == site.device)
                {
                    self.visit(site, &child, &relative.join(&name), depth + 1)?;
                }
            } else if is_exact_temporary_name(&name) {
                self.quarantine(site, dir, relative, &name);
            }
        }
        Ok(())
    }

    /// 一层目录的名字和是不是真目录（不跟随链接）。读不了返回 `None`，这一层原地不动。
    fn list(&mut self, dir: &Dir) -> Result<Option<Vec<(String, bool)>>, OutOfBudget> {
        let Ok(entries) = dir.entries() else {
            return Ok(None);
        };
        let mut listed = Vec::new();
        for entry in entries {
            if self.remaining == 0 {
                return Err(OutOfBudget);
            }
            self.remaining -= 1;
            self.in_time()?;
            let Ok(entry) = entry else {
                return Ok(None);
            };
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            // 不是 UTF-8 的名字不是 Broker 起的，也不往里走。
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            listed.push((name, file_type.is_dir()));
        }
        Ok(Some(listed))
    }

    fn in_time(&self) -> Result<(), OutOfBudget> {
        if Instant::now() >= self.deadline {
            return Err(OutOfBudget);
        }
        Ok(())
    }

    /// 把一个名字对得上的残留挪进本处隔离区；不像 Broker 自己写下的就原地不动。
    fn quarantine(&self, site: &mut Site<'_>, dir: &Dir, relative: &Path, name: &str) {
        let Ok(metadata) = dir.symlink_metadata(name) else {
            return;
        };
        // 原子写起的是本用户的普通单链接文件；链接的 is_file 为假。
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != self.uid
            || metadata.dev() != site.device
        {
            return;
        }
        let Some(target) = site.target(relative) else {
            return;
        };
        if rename_no_replace(dir, name, &target).is_err() {
            return;
        }
        // 改名已经生效；落盘失败只影响崩溃后它停在哪一边，两边都不会丢。
        let _ = FsVaultBroker::sync_directory(&target, Path::new("."));
        let _ = FsVaultBroker::sync_directory(dir, Path::new("."));
    }
}

impl Site<'_> {
    /// 本批次里对应原相对路径的目录，逐段建；建不成返回 `None`。
    fn target(&mut self, relative: &Path) -> Option<Dir> {
        if matches!(self.batch, Batch::Untried) {
            self.batch = create_batch(self.base).map_or(Batch::Failed, Batch::Ready);
        }
        let Batch::Ready(batch) = &self.batch else {
            return None;
        };
        let mut target = batch.try_clone().ok()?;
        for component in relative.components() {
            target = ensure_directory(&target, component.as_os_str().to_str()?)?;
        }
        Some(target)
    }
}

/// 建本次的隔离批次 `residue-quarantine/<毫秒>-<进程号>-<序号>`；名字被占了就换下一个序号。
fn create_batch(base: &Dir) -> Option<Dir> {
    let quarantine = ensure_directory(base, QUARANTINE)?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    for _ in 0..16 {
        let name = format!(
            "{millis}-{}-{}",
            std::process::id(),
            BATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        match mkdirat(&quarantine, name.as_str(), Mode::RWXU) {
            Ok(()) => {
                FsVaultBroker::sync_directory(&quarantine, Path::new(".")).ok()?;
                return directory(&quarantine, &name).ok();
            }
            Err(error) if error == Errno::EXIST => {}
            Err(_) => return None,
        }
    }
    None
}

/// 建一层只有本用户能进的目录，或复用已有的真目录；打开不跟随链接，同名的链接和文件都打不开。
fn ensure_directory(parent: &Dir, name: &str) -> Option<Dir> {
    match mkdirat(parent, name, Mode::RWXU) {
        Ok(()) => FsVaultBroker::sync_directory(parent, Path::new(".")).ok()?,
        Err(error) if error == Errno::EXIST => {}
        Err(_) => return None,
    }
    directory(parent, name).ok()
}

/// 改名不覆盖：Linux 上用 `RENAME_NOREPLACE`。
#[cfg(target_os = "linux")]
fn rename_no_replace(from: &Dir, name: &str, to: &Dir) -> rustix::io::Result<()> {
    rustix::fs::renameat_with(from, name, to, name, rustix::fs::RenameFlags::NOREPLACE)
}

/// 改名不覆盖：别处先硬链接再删原名，同样不会盖掉已有的东西；删不掉时两边都在，原处照旧挡备份。
#[cfg(not(target_os = "linux"))]
fn rename_no_replace(from: &Dir, name: &str, to: &Dir) -> std::io::Result<()> {
    from.hard_link(name, to, name)?;
    from.remove_file(name)
}

#[cfg(test)]
#[path = "../tests/residue_private/mod.rs"]
mod tests;
