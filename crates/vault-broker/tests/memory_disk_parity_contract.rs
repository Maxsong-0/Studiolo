//! 内存替身与磁盘实现的终态一致性契约。
//!
//! `MemoryVaultBroker` 只有一个存在理由：让不便起磁盘 Vault 的上层契约测试仍然跑在
//! 与生产一致的语义上。一旦两者在某条边界上给出不同终态，基于替身的测试就会给出磁盘
//! 拿不到的信心——这类缺陷比普通 bug 更危险，因为它让红灯永远不亮。
//!
//! 因此本文件同时钉死两件事：
//! 1. **绝对行为**：每条边界场景的终态形状必须等于下表写死的期望，两侧各自比一遍，
//!    避免「两边一起漂移」仍然算通过。
//! 2. **替身一致性**：两侧逐条互比，任何新增分叉立刻红灯。
//!
//! 场景以「形状」而非原样错误比较：`Revision` 含 Vault UUID，两个 adapter 的 UUID 天然
//! 不同，逐字节比较只会比出无意义的差异。形状保留变体、逻辑路径与不可翻译的原因串。

use studiolo_core::{AttemptId, Fence, TaskId, VaultPath};
use studiolo_vault_broker::{
    ArtifactCandidate, ArtifactCommit, BrokerError, FsVaultBroker, MemoryVaultBroker, Mutation,
    SnapshotInput, VaultBroker,
};
use tempfile::TempDir;

/// 每条场景的期望终态形状。顺序与 `scenarios` 的记录顺序一一对应。
const EXPECTED: &[(&str, &str)] = &[
    // —— 读路径：命名空间判定必须与 `FsVaultBroker::validate_components` 同形 ——
    ("read-missing", "NotFound(missing.md)"),
    ("read-directory", "UnsafeNode(dir|not a regular file)"),
    (
        "read-under-file",
        "UnsafeNode(file.md/child.md|non-directory path component)",
    ),
    ("read-alias-leaf", "PathCollision(File.md|file.md)"),
    // 读路径的 `existing` 带父前缀，写路径不带；这条差异是磁盘固有的，替身必须照抄。
    ("read-alias-parent", "PathCollision(Dir/leaf.md|dir)"),
    ("read-identity-marker", "ok"),
    ("read-deep-missing", "NotFound(a/b/c/d.md)"),
    // —— 写路径：命名空间判定必须与 `blob::paths` 的逐级观察同形 ——
    ("write-over-directory", "UnsafeNode(dir|not a regular file)"),
    // 父链上出现普通文件时磁盘在 `writer::lease::child` 失败，替身照抄该 I/O 错误。
    ("write-under-file", "Io(open writer layout|NotADirectory)"),
    ("write-alias-leaf", "PathCollision(File.md|file.md)"),
    ("write-alias-parent", "PathCollision(Dir/other.md|dir)"),
    (
        "write-protected-workbench",
        "ProtectedPath(.workbench/x.json)",
    ),
    ("write-protected-git", "ProtectedPath(.git/x)"),
    (
        "write-empty-key",
        "idempotency key was reused with different input",
    ),
    ("write-oversize", "writer storage or input limit exceeded"),
    ("write-too-deep", "writer storage or input limit exceeded"),
    // —— CAS 与重放终态 ——
    (
        "write-cas-mismatch",
        "RevisionConflict(file.md|absence|present=true)",
    ),
    (
        "write-replay-rejection",
        "RevisionConflict(file.md|absence|present=true)",
    ),
    (
        "write-replay-different-input",
        "idempotency key was reused with different input",
    ),
    ("write-replay-success", "ok"),
    // —— 快照 ——
    (
        "snapshot-duplicate",
        "InvalidArtifact(snapshot contains duplicate paths)",
    ),
    (
        "snapshot-stale-revision",
        "RevisionConflict(file.md|rev|present=true)",
    ),
    ("snapshot-missing-path", "NotFound(nope.md)"),
    ("snapshot-empty", "ok"),
    ("snapshot-roundtrip", "ok"),
    // —— 产物提交 ——
    (
        "artifact-reserved-name",
        "InvalidArtifact(reserved artifact name: manifest.json)",
    ),
    (
        "artifact-colliding-names",
        "InvalidArtifact(artifact names collide)",
    ),
    ("artifact-commit", "ok"),
    ("artifact-replay", "ok"),
    (
        "artifact-different-content-same-key",
        "idempotency key was reused with different input",
    ),
    (
        "artifact-empty-intent",
        "InvalidArtifact(audit intent and idempotency key are required)",
    ),
];

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).expect("测试路径必须合法")
}

/// 把失败折叠成不含 Vault UUID 的稳定形状；无法翻译的变体退回 `Display`。
fn shape<T>(result: Result<T, BrokerError>) -> String {
    let error = match result {
        Ok(_) => return "ok".to_owned(),
        Err(error) => error,
    };
    match error {
        BrokerError::NotFound(path) => format!("NotFound({path})"),
        BrokerError::PathCollision {
            requested,
            existing,
        } => format!("PathCollision({requested}|{existing})"),
        BrokerError::UnsafeNode { path, reason } => format!("UnsafeNode({path}|{reason})"),
        BrokerError::RevisionConflict {
            path,
            expected,
            actual,
        } => format!(
            "RevisionConflict({path}|{}|present={})",
            if expected == "absence" {
                "absence"
            } else {
                "rev"
            },
            actual.is_some()
        ),
        BrokerError::Io { operation, source } => format!("Io({operation}|{:?})", source.kind()),
        BrokerError::Json { operation, .. } => format!("Json({operation})"),
        BrokerError::InvalidArtifact(message) => format!("InvalidArtifact({message})"),
        BrokerError::ArtifactConflict(path) => format!("ArtifactConflict({path})"),
        BrokerError::ProtectedPath(path) => format!("ProtectedPath({path})"),
        other => other.to_string(),
    }
}

fn artifact_commit(
    task_id: TaskId,
    attempt_id: AttemptId,
    artifacts: &[(&str, &[u8])],
    idempotency_key: &str,
) -> ArtifactCommit {
    ArtifactCommit {
        task_id,
        attempt_id,
        fence: Fence::new(1).expect("非零 fence"),
        audit_intent: "parity-intent".to_owned(),
        idempotency_key: idempotency_key.to_owned(),
        artifacts: artifacts
            .iter()
            .map(|(name, content)| ArtifactCandidate {
                name: path(name),
                content: content.to_vec(),
            })
            .collect(),
    }
}

/// 在一个空 Vault 上按固定顺序跑完整组边界场景，返回逐条终态形状。
fn scenarios(broker: &mut dyn VaultBroker) -> Vec<(&'static str, String)> {
    let mut observed: Vec<(&'static str, String)> = Vec::new();

    observed.push(("read-missing", shape(broker.read(&path("missing.md")))));
    broker
        .mutate(Mutation::create(
            path("dir/leaf.md"),
            b"leaf".to_vec(),
            "seed-dir",
        ))
        .expect("建立目录夹具");
    observed.push(("read-directory", shape(broker.read(&path("dir")))));
    broker
        .mutate(Mutation::create(
            path("file.md"),
            b"file".to_vec(),
            "seed-file",
        ))
        .expect("建立文件夹具");
    observed.push((
        "read-under-file",
        shape(broker.read(&path("file.md/child.md"))),
    ));
    observed.push(("read-alias-leaf", shape(broker.read(&path("File.md")))));
    observed.push((
        "read-alias-parent",
        shape(broker.read(&path("Dir/leaf.md"))),
    ));
    observed.push((
        "read-identity-marker",
        shape(broker.read(&path(".workbench/vault.json"))),
    ));
    observed.push(("read-deep-missing", shape(broker.read(&path("a/b/c/d.md")))));

    observed.push((
        "write-over-directory",
        shape(broker.mutate(Mutation::create(path("dir"), b"x".to_vec(), "w1"))),
    ));
    observed.push((
        "write-under-file",
        shape(broker.mutate(Mutation::create(
            path("file.md/child.md"),
            b"x".to_vec(),
            "w2",
        ))),
    ));
    observed.push((
        "write-alias-leaf",
        shape(broker.mutate(Mutation::create(path("File.md"), b"x".to_vec(), "w3"))),
    ));
    observed.push((
        "write-alias-parent",
        shape(broker.mutate(Mutation::create(path("Dir/other.md"), b"x".to_vec(), "w4"))),
    ));
    observed.push((
        "write-protected-workbench",
        shape(broker.mutate(Mutation::create(
            path(".workbench/x.json"),
            b"x".to_vec(),
            "w5",
        ))),
    ));
    observed.push((
        "write-protected-git",
        shape(broker.mutate(Mutation::create(path(".git/x"), b"x".to_vec(), "w6"))),
    ));
    observed.push((
        "write-empty-key",
        shape(broker.mutate(Mutation::create(path("empty.md"), b"x".to_vec(), ""))),
    ));
    observed.push((
        "write-oversize",
        shape(broker.mutate(Mutation::create(
            path("big.md"),
            vec![0u8; 4 * 1024 * 1024 + 1],
            "w7",
        ))),
    ));
    let deep = (0..70)
        .map(|index| format!("s{index}"))
        .collect::<Vec<_>>()
        .join("/");
    observed.push((
        "write-too-deep",
        shape(broker.mutate(Mutation::create(
            path(&format!("{deep}/leaf.md")),
            b"x".to_vec(),
            "w8",
        ))),
    ));

    observed.push((
        "write-cas-mismatch",
        shape(broker.mutate(Mutation::create(path("file.md"), b"other".to_vec(), "w9"))),
    ));
    observed.push((
        "write-replay-rejection",
        shape(broker.mutate(Mutation::create(path("file.md"), b"other".to_vec(), "w9"))),
    ));
    observed.push((
        "write-replay-different-input",
        shape(broker.mutate(Mutation::create(
            path("file.md"),
            b"different".to_vec(),
            "w9",
        ))),
    ));
    observed.push((
        "write-replay-success",
        shape(broker.mutate(Mutation::create(
            path("file.md"),
            b"file".to_vec(),
            "seed-file",
        ))),
    ));

    let file = broker.read(&path("file.md")).expect("读回文件夹具");
    let leaf = broker.read(&path("dir/leaf.md")).expect("读回目录夹具");
    observed.push((
        "snapshot-duplicate",
        shape(broker.snapshot(&[
            SnapshotInput {
                path: file.path.clone(),
                expected_revision: file.revision,
            },
            SnapshotInput {
                path: file.path.clone(),
                expected_revision: file.revision,
            },
        ])),
    ));
    observed.push((
        "snapshot-stale-revision",
        shape(broker.snapshot(&[SnapshotInput {
            path: file.path.clone(),
            expected_revision: leaf.revision,
        }])),
    ));
    observed.push((
        "snapshot-missing-path",
        shape(broker.snapshot(&[SnapshotInput {
            path: path("nope.md"),
            expected_revision: file.revision,
        }])),
    ));
    observed.push(("snapshot-empty", shape(broker.snapshot(&[]))));
    let captured = broker
        .snapshot(&[SnapshotInput {
            path: file.path.clone(),
            expected_revision: file.revision,
        }])
        .expect("捕获快照");
    observed.push((
        "snapshot-roundtrip",
        shape(broker.load_snapshot(&captured.id)),
    ));

    let task_id = TaskId::new();
    let attempt_id = AttemptId::new();
    observed.push((
        "artifact-reserved-name",
        shape(broker.commit_attempt_artifacts(artifact_commit(
            task_id,
            attempt_id,
            &[("manifest.json", b"x")],
            "c1",
        ))),
    ));
    observed.push((
        "artifact-colliding-names",
        shape(broker.commit_attempt_artifacts(artifact_commit(
            task_id,
            attempt_id,
            &[("Out.md", b"x"), ("out.md", b"y")],
            "c2",
        ))),
    ));
    observed.push((
        "artifact-commit",
        shape(broker.commit_attempt_artifacts(artifact_commit(
            task_id,
            attempt_id,
            &[("out.md", b"x")],
            "c3",
        ))),
    ));
    observed.push((
        "artifact-replay",
        shape(broker.commit_attempt_artifacts(artifact_commit(
            task_id,
            attempt_id,
            &[("out.md", b"x")],
            "c3",
        ))),
    ));
    observed.push((
        "artifact-different-content-same-key",
        shape(broker.commit_attempt_artifacts(artifact_commit(
            task_id,
            attempt_id,
            &[("out.md", b"changed")],
            "c3",
        ))),
    ));
    observed.push((
        "artifact-empty-intent",
        shape(broker.commit_attempt_artifacts(ArtifactCommit {
            audit_intent: String::new(),
            ..artifact_commit(task_id, attempt_id, &[("out.md", b"x")], "c4")
        })),
    ));

    observed
}

fn disk_broker(temp: &TempDir) -> FsVaultBroker {
    FsVaultBroker::open(&temp.path().join("vault"), &temp.path().join("data"))
        .expect("打开磁盘 Vault")
}

fn temp_root() -> TempDir {
    TempDir::new_in(
        std::env::temp_dir()
            .canonicalize()
            .expect("规范化系统临时目录"),
    )
    .expect("创建测试根目录")
}

fn assert_matches_expected(adapter: &str, observed: &[(&'static str, String)]) {
    assert_eq!(
        observed.len(),
        EXPECTED.len(),
        "{adapter}：场景条数与期望表不符，新增场景必须同时登记期望"
    );
    for ((name, actual), (expected_name, expected)) in observed.iter().zip(EXPECTED) {
        assert_eq!(name, expected_name, "{adapter}：场景顺序与期望表不符");
        assert_eq!(actual, expected, "{adapter}：场景 {name} 的终态与期望不符");
    }
}

#[test]
fn the_memory_adapter_produces_the_documented_outcome_for_every_boundary_scenario() {
    assert_matches_expected("内存替身", &scenarios(&mut MemoryVaultBroker::new()));
}

#[test]
fn the_filesystem_adapter_produces_the_documented_outcome_for_every_boundary_scenario() {
    let temp = temp_root();
    assert_matches_expected("磁盘实现", &scenarios(&mut disk_broker(&temp)));
}

#[test]
fn the_memory_adapter_and_the_filesystem_adapter_agree_on_every_boundary_scenario() {
    let memory = scenarios(&mut MemoryVaultBroker::new());
    let temp = temp_root();
    let disk = scenarios(&mut disk_broker(&temp));
    assert_eq!(memory.len(), disk.len(), "两侧跑过的场景条数必须一致");
    let diverged: Vec<String> = memory
        .iter()
        .zip(disk.iter())
        .filter(|((_, left), (_, right))| left != right)
        .map(|((name, left), (_, right))| format!("{name}: 内存={left} 磁盘={right}"))
        .collect();
    assert!(
        diverged.is_empty(),
        "替身与磁盘的终态出现分叉：{}",
        diverged.join("；")
    );
}

/// 拒绝是终态：ABA 让 CAS 前提重新成立之后，重放同一幂等键仍须返回原拒绝，
/// 而不是被「升级」成一次成功安装。两侧必须都守住这条，否则替身会放行磁盘挡住的写。
fn assert_a_replayed_rejection_survives_an_aba_cycle(broker: &mut dyn VaultBroker) {
    let target = path("aba.md");
    let first = broker
        .mutate(Mutation::create(
            target.clone(),
            b"one".to_vec(),
            "aba-seed",
        ))
        .expect("建立 ABA 夹具")
        .revision;
    let second = broker
        .mutate(Mutation::replace(
            target.clone(),
            b"two".to_vec(),
            first,
            "aba-advance",
        ))
        .expect("推进到中间 revision")
        .revision;

    // 此刻当前 revision 是 second，要求 first 必然被拒绝，并被记成终态。
    let rejection = broker.mutate(Mutation::replace(
        target.clone(),
        b"three".to_vec(),
        first,
        "aba-rejected",
    ));
    let BrokerError::RevisionConflict { actual, .. } = rejection.expect_err("过期前提必须被拒绝")
    else {
        panic!("过期 CAS 前提必须返回 RevisionConflict");
    };
    assert_eq!(actual, Some(second), "拒绝须记录被拒时观察到的 revision");

    // A→B→A：内容回到 one，first 这个 revision 重新成为当前值。
    let restored = broker
        .mutate(Mutation::replace(
            target.clone(),
            b"one".to_vec(),
            second,
            "aba-restore",
        ))
        .expect("回到初始内容")
        .revision;
    assert_eq!(restored, first, "内容寻址的 revision 在 ABA 之后应当复现");

    let replayed = broker.mutate(Mutation::replace(
        target,
        b"three".to_vec(),
        first,
        "aba-rejected",
    ));
    let BrokerError::RevisionConflict { actual, .. } =
        replayed.expect_err("已拒绝的幂等键重放仍须被拒绝")
    else {
        panic!("重放已拒绝的幂等键必须返回原 RevisionConflict");
    };
    assert_eq!(
        actual,
        Some(second),
        "重放须返回原拒绝记录的 revision，而不是当下重新观察的值"
    );
}

#[test]
fn the_memory_adapter_keeps_a_rejection_terminal_across_an_aba_cycle() {
    assert_a_replayed_rejection_survives_an_aba_cycle(&mut MemoryVaultBroker::new());
}

#[test]
fn the_filesystem_adapter_keeps_a_rejection_terminal_across_an_aba_cycle() {
    let temp = temp_root();
    assert_a_replayed_rejection_survives_an_aba_cycle(&mut disk_broker(&temp));
}
