//! 部署诊断失败关闭，全程不启动任何系统服务或供应商 CLI。
use studiolo_core::{AttemptId, Fence, TaskId};
use studiolo_runner_adapter::{
    AdapterAvailability, CliProvider, ExecutionSpec, LinuxIsolation, RealRunner, RunBudget,
    RunSpec, RunnerAdapter, RunnerError, RunnerLaunchConfig, SourceEgress,
};

#[test]
fn absent_deployment_has_explicit_diagnostics_and_is_not_a_retryable_launch_failure() {
    for provider in [CliProvider::Claude, CliProvider::Codex] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let runner = RealRunner::new(RunnerLaunchConfig {
            provider,
            cli_sha256: "0".repeat(64),
            isolation: LinuxIsolation {
                runtime_root: root.join("missing-runtime"),
                bubblewrap: root.join("missing-bwrap"),
                systemd_run: root.join("missing-systemd"),
                gateway_socket: root.join("missing-socket"),
            },
            source_checkout: root.into(),
            attempt_root: root.into(),
            retained_root: root.into(),
            capability_file: root.join("unused-ticket"),
        });
        let report = runner.probe_report().unwrap();
        assert!(matches!(
            report.availability,
            AdapterAvailability::Unavailable { .. }
        ));
        assert!(report.binary_version.is_none());
        assert!(report.protocol_capabilities.is_empty());
        assert!(!report.missing_prerequisites.is_empty());
        let mut spec = RunSpec {
            task_id: TaskId::new(),
            attempt_id: AttemptId::new(),
            fence: Fence::new(1).unwrap(),
            instruction: "fixture".into(),
            inputs: vec![],
            budget: RunBudget::default(),
            execution: Some(ExecutionSpec {
                snapshot_id: "a".repeat(64),
                policy_hash: "b".repeat(64),
                provider,
                model: "fixture-model".into(),
                input_egress: vec![],
                repository_tree: "c".repeat(40),
                repository_egress: SourceEgress::Providers(vec![provider]),
            }),
        };
        assert!(matches!(runner.start(&spec), Err(RunnerError::Unavailable)));
        for classification in [SourceEgress::Deny, SourceEgress::LocalOnly] {
            spec.execution.as_mut().unwrap().repository_egress = classification;
            assert!(matches!(
                runner.start(&spec),
                Err(RunnerError::InvalidExecution)
            ));
        }
        assert_eq!(std::fs::read_dir(root).unwrap().count(), 0);
    }
}
