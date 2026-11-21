//! T-216 独立复查第 1 轮 F16：回收或收尾删登记目录删到一半停下（期限到、I/O 出错、进程被杀，都停在两步之间），
//! 下一轮回收要认得出、接着删完。步数预算只在这里用，停在任意一步之后；生产里只有截止时间。
use super::*;
use crate::checkpoint::{
    manifest::{AuditCoverageProof, DeploymentEntry, ManifestInputs, PackageDigest},
    staging::{SealInputs, SourceCatalog, StagingArea, housekeeping::reclaim, reclaim_staging},
    types::OwnerRoster,
};
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::PathBuf,
    time::Duration,
};
use studiolo_core::checkpoint::{
    CheckpointOwner, InstantMillis, OwnerAssembly, OwnerWatermark, SealedOwnerCheckpoint,
    WalCheckpointProof,
};

const DIGEST: &str = "ab12cd34ef56ab78cd90ef12ab34cd56ef78ab90cd12ef34ab56cd78ef90ab12";
const MARKERS: [&str; 3] = ["incomplete-", "retained-", "removing-"];

struct Fixture {
    _temp: tempfile::TempDir,
    parent: PathBuf,
    source: PathBuf,
    ephemeral: PathBuf,
    reader: Option<u32>,
}
impl Fixture {
    fn new(shared: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let [parent, source, ephemeral] =
            ["staging", "source", "ephemeral"].map(|name| temp.path().join(name));
        for path in [&parent, &source, &ephemeral] {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let reader = shared.then(|| {
            fs::set_permissions(&parent, fs::Permissions::from_mode(0o750)).unwrap();
            fs::metadata(&parent).unwrap().gid()
        });
        Self {
            _temp: temp,
            parent,
            source,
            ephemeral,
            reader,
        }
    }
    fn area(&self, id: &str) -> StagingArea {
        self.try_area(id).unwrap()
    }
    fn try_area(&self, id: &str) -> Result<StagingArea> {
        let id = CheckpointId::parse(id).unwrap();
        let sources = std::slice::from_ref(&self.source);
        match self.reader {
            Some(gid) => StagingArea::create_for_reader(
                &self.parent,
                id,
                sources,
                &self.ephemeral,
                gid,
                deadline(),
            ),
            None => StagingArea::create(&self.parent, id, sources, &self.ephemeral, deadline()),
        }
    }
    /// 父目录里的名字；登记目录后面带上里面剩下的文件。
    fn layout(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.parent)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let name = entry.file_name().into_string().unwrap();
                if !MARKERS.iter().any(|prefix| name.starts_with(prefix)) {
                    return name;
                }
                let mut files: Vec<String> = fs::read_dir(entry.path())
                    .unwrap()
                    .map(|file| file.unwrap().file_name().into_string().unwrap())
                    .collect();
                files.sort();
                format!("{name}{files:?}")
            })
            .collect();
        names.sort();
        names
    }
    /// 登记目录里还剩几个文件；没有登记目录时为空。
    fn marker_files(&self) -> Option<usize> {
        fs::read_dir(&self.parent)
            .unwrap()
            .map(|entry| entry.unwrap())
            .find(|entry| {
                let name = entry.file_name().into_string().unwrap();
                MARKERS.iter().any(|prefix| name.starts_with(prefix))
            })
            .map(|entry| fs::read_dir(entry.path()).unwrap().count())
    }
    /// 父目录下整棵树的相对路径：删深处的一个文件、改一次名都看得出来。
    fn tree(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        let mut pending = vec![self.parent.clone()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(&directory).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    pending.push(entry.path());
                }
                paths.push(entry.path().strip_prefix(&self.parent).unwrap().to_owned());
            }
        }
        paths.sort();
        paths
    }
    fn only_lease(&self) -> bool {
        self.layout() == [".producer-lease"]
    }
    /// 已改名、删到一半的登记目录。
    fn removing(&self) -> Option<PathBuf> {
        fs::read_dir(&self.parent)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("removing-")
            })
    }
    /// 每轮回收只让它做一步，直到登记目录已改名、里面正好剩 `left` 个文件；返回改名后的路径。
    fn removing_with(&self, left: usize) -> PathBuf {
        for _ in 0..10_000 {
            if let Some(path) = self.removing()
                && fs::read_dir(&path).unwrap().count() == left
            {
                return path;
            }
            let _ = reclaim(&self.parent, self.reader, &mut Budget::steps(deadline(), 1));
        }
        panic!("没走到剩 {left} 个文件的那一步：{:?}", self.layout());
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
fn raw(area: &StagingArea) {
    let path = area.export_root().join("canonical/raw");
    fs::write(&path, b"unsealed raw").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
}
/// 进程在这里没了：不再跑 Drop 里的回收，只有内核锁跟着进程放下。
fn die(mut area: StagingArea) {
    area.lifecycle.finished = true;
    drop(area);
}
/// 只装了 Audit 一个 owner 的最小封存输入，到期时刻由调用方给。
fn inputs(expires_at: InstantMillis) -> SealInputs {
    let audit = SealedOwnerCheckpoint {
        owner: CheckpointOwner::Audit,
        schema_version: 1,
        wal: WalCheckpointProof {
            busy: 0,
            log_frames: 1,
            checkpointed_frames: 1,
        },
        target_synced: true,
        parent_directory_synced: true,
        reopen_verified: true,
        source_binding_stable: true,
        watermark: OwnerWatermark::Audit {
            last_offset: 1,
            last_hash: DIGEST.to_owned(),
            agent_ledger: None,
        },
    };
    let package = |name: &str| PackageDigest {
        name: name.to_owned(),
        version: "0.0.1".to_owned(),
        digest: DIGEST.to_owned(),
    };
    let roster = CheckpointOwner::FREEZE_ORDER.iter().map(|owner| {
        let assembly = if *owner == CheckpointOwner::Audit {
            OwnerAssembly::Present
        } else {
            OwnerAssembly::Absent
        };
        (*owner, assembly)
    });
    SealInputs {
        roster: OwnerRoster::new(roster).unwrap(),
        owners: vec![audit],
        manifest: ManifestInputs {
            application_packages: vec![package("studiolo-local-server")],
            cli_packages: vec![package("studiolo-auth-admin")],
            deployment: vec![DeploymentEntry {
                unit: "studiolo-local-server.service".to_owned(),
                revision: "r1".to_owned(),
            }],
            secret_references: Vec::new(),
            audit_coverage: AuditCoverageProof {
                acked_events_checked: 1,
                acked_events_matched: 1,
                offset_comparison_only: false,
            },
        },
        content_cutoff: InstantMillis::from_millis(900),
        now: InstantMillis::from_millis(1000),
        expires_at,
    }
}
/// 到期时刻在 1970 年的封存：回收一看就是过期的。
fn sealed(f: &Fixture, id: &str) -> SealedCheckpoint {
    sealed_until(f, id, InstantMillis::from_millis(9000))
}
fn sealed_until(f: &Fixture, id: &str, expires_at: InstantMillis) -> SealedCheckpoint {
    f.area(id)
        .seal(&SourceCatalog::new([]).unwrap(), inputs(expires_at))
        .unwrap()
}
fn tomorrow() -> InstantMillis {
    InstantMillis::from_millis(journal::now().unwrap().millis() + 86_400_000)
}
/// 从第 0 步起一步步加大预算：每次在新做的现场上让 `stop` 停在那一步之后，再跑一轮正常回收，
/// 要求这一轮成功、只剩租约、之后能起新的尝试。预算够了不再停下时结束，返回途中见过的登记目录文件数。
fn every_stop(
    label: &str,
    shared: bool,
    stop: impl Fn(&Fixture, u64) -> Result<()>,
) -> BTreeSet<usize> {
    let mut seen = BTreeSet::new();
    let mut failures = Vec::new();
    for steps in 0..10_000 {
        let f = Fixture::new(shared);
        if stop(&f, steps).is_ok() {
            assert!(
                f.only_lease(),
                "{label}：不停的那一次没收干净 {:?}",
                f.layout()
            );
            assert!(
                failures.is_empty(),
                "{label}：{} 个停点之后下一轮收不干净：\n{}",
                failures.len(),
                failures.join("\n")
            );
            return seen;
        }
        seen.extend(f.marker_files());
        let stopped = f.layout();
        // 已改了名的按「续删」记，还叫原名的按原来的类别记，每轮正好收走这一份。
        let resumed = u32::from(stopped.iter().any(|name| name.starts_with("removing-")));
        match reclaim_staging(&f.parent, f.reader, deadline()) {
            Ok(summary)
                if f.only_lease()
                    && summary.resumed == resumed
                    && summary.interrupted + summary.expired + summary.resumed == 1
                    && summary.retained == 0 =>
            {
                f.area("next").abort().unwrap();
            }
            next => failures.push(format!(
                "  停在第 {steps} 步之后 {stopped:?} → 下一轮 {next:?}，剩下 {:?}",
                f.layout()
            )),
        }
    }
    panic!("{label}：一万步都没做完");
}
fn covers(seen: &BTreeSet<usize>, files: usize) -> bool {
    (0..=files).all(|left| seen.contains(&left))
}
#[test]
fn reclaim_stopped_after_any_step_of_an_interrupted_attempt_finishes_next_round() {
    for shared in [false, true] {
        let seen = every_stop("中断的尝试，回收停下", shared, |f, steps| {
            let area = f.area("interrupted");
            raw(&area);
            die(area);
            reclaim(&f.parent, f.reader, &mut Budget::steps(deadline(), steps)).map(drop)
        });
        // identity、export、pending 三份登记：删掉其中每一份之后、删目录之前都停过。
        assert!(covers(&seen, 3), "{seen:?}");
    }
}
#[test]
fn reclaim_stopped_after_any_step_of_an_expired_seal_finishes_next_round() {
    for shared in [false, true] {
        let seen = every_stop("到期的封存，回收停下", shared, |f, steps| {
            sealed(f, "expired");
            reclaim(&f.parent, f.reader, &mut Budget::steps(deadline(), steps)).map(drop)
        });
        assert!(covers(&seen, 4), "{seen:?}");
    }
}
#[test]
fn abort_stopped_after_any_step_finishes_on_the_next_reclaim() {
    for shared in [false, true] {
        let seen = every_stop("放弃的尝试，收尾停下", shared, |f, steps| {
            let mut area = f.area("aborted");
            raw(&area);
            let result = area.cleanup_within(&mut Budget::steps(deadline(), steps));
            die(area);
            result
        });
        assert!(covers(&seen, 3), "{seen:?}");
    }
}
/// 封存改名成 `retained-` 之后、交还引用之前出了错（改名后父目录 fsync 失败、之后的复核失败）：
/// 照 `seal` 的真实顺序做到那一步，这次的封存根 `sealed-` 在场、里面有文件，期限还没到。
fn retain_failed_after_rename(f: &Fixture) -> StagingArea {
    let donor = Fixture::new(false);
    let sealed = sealed_until(&donor, "renamed", tomorrow());
    let mut area = f.area("renamed");
    raw(&area);
    fs::write(f.parent.join(&area.pending_name).join("data/kept"), b"view").unwrap();
    area.exports
        .discard_until(&area.parent, &area.export_name, None, deadline())
        .unwrap();
    area.lifecycle.published = true;
    area.parent
        .publish(&area.pending_name, &area.final_name)
        .unwrap();
    area.lifecycle
        .retain(&area.parent, &sealed, deadline())
        .unwrap();
    area.lifecycle.finished = false;
    area
}
/// 失败回收停在任意一步之后，下一轮回收都过得去：什么都还没删的是一份完整的封存，照常留到到期；
/// 删了任何东西的，这一轮要收干净（未到期的 `retained-` 要求封存根在场，所以删根之前得先改回原名）。
#[test]
fn rollback_after_the_retained_rename_stopped_after_any_step_never_blocks_reclaim() {
    let mut seen = BTreeSet::new();
    let mut failures = Vec::new();
    for steps in 0..10_000 {
        let f = Fixture::new(false);
        let mut area = retain_failed_after_rename(&f);
        let before = f.tree();
        let result = area.cleanup_within(&mut Budget::steps(deadline(), steps));
        die(area);
        if result.is_ok() {
            assert!(f.only_lease(), "{:?}", f.layout());
            assert!(failures.is_empty(), "{}", failures.join("\n"));
            assert!(covers(&seen, 4), "{seen:?}");
            return;
        }
        seen.extend(f.marker_files());
        let stopped = f.layout();
        let untouched = f.tree() == before;
        let next = reclaim_staging(&f.parent, f.reader, deadline());
        let fine = match &next {
            Ok(summary) if untouched => summary.retained == 1 && f.tree() == before,
            Ok(summary) => f.only_lease() && summary.interrupted + summary.resumed == 1,
            Err(_) => false,
        };
        if !fine {
            failures.push(format!(
                "  停在第 {steps} 步之后 {stopped:?} → 下一轮 {next:?}，剩下 {:?}",
                f.layout()
            ));
        }
    }
    panic!("一万步都没做完");
}
/// 改名其实没生效（`retain` 在改名那一步就失败了）：登记还叫原名，失败回收照常删完，不去改一个不存在的名字。
#[test]
fn rollback_when_the_retained_rename_never_took_effect_still_cleans_up() {
    let f = Fixture::new(false);
    let mut area = retain_failed_after_rename(&f);
    let retained = area.lifecycle.retained_name.clone().unwrap();
    fs::rename(
        f.parent.join(&retained),
        f.parent.join(&area.lifecycle.name),
    )
    .unwrap();
    area.cleanup().unwrap();
    die(area);
    assert!(f.only_lease(), "{:?}", f.layout());
}
/// 同一进程里接着收尾（`abort` 失败后 Drop 再来一次）也要从停下的地方接着删完。
#[test]
fn abort_stopped_after_any_step_finishes_when_retried_in_the_same_process() {
    let mut failures = Vec::new();
    for steps in 0..10_000 {
        let f = Fixture::new(false);
        let mut area = f.area("retried");
        raw(&area);
        if area
            .cleanup_within(&mut Budget::steps(deadline(), steps))
            .is_ok()
        {
            assert!(f.only_lease(), "{:?}", f.layout());
            assert!(failures.is_empty(), "{}", failures.join("\n"));
            return;
        }
        let stopped = f.layout();
        let retried = area.abort();
        if retried.is_err() || !f.only_lease() {
            failures.push(format!(
                "  停在第 {steps} 步之后 {stopped:?} → 重试 {retried:?}，剩下 {:?}",
                f.layout()
            ));
        }
    }
    panic!("一万步都没做完");
}
/// 每轮只让回收做一步（每一步之后都停下、下一轮接着做），最后也收干净，且每轮都有进展。
#[test]
fn reclaim_doing_one_step_per_round_still_finishes() {
    for shared in [false, true] {
        let f = Fixture::new(shared);
        sealed(&f, "expired");
        let area = f.area("interrupted");
        raw(&area);
        die(area);
        let mut rounds = 0;
        while !f.only_lease() {
            let before = f.tree();
            let _ = reclaim(&f.parent, f.reader, &mut Budget::steps(deadline(), 1));
            assert_ne!(f.tree(), before, "第 {rounds} 轮没有进展");
            rounds += 1;
            assert!(rounds < 10_000);
        }
        f.area("next").abort().unwrap();
    }
}
#[path = "staging_retire_refusals.rs"]
mod refusals;
