//! Read-only deployment probe; does not start a model or read a provider credential.
use studiolo_runner_adapter::{
    CliProvider, LinuxIsolation, RealRunner, RunnerAdapter, RunnerLaunchConfig,
};
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 6 {
        eprintln!("usage: probe claude|codex ROOTFS CLI_SHA256 SCRATCH RETAINED GATEWAY_SOCKET");
        std::process::exit(2);
    }
    let provider = match args[0].as_str() {
        "claude" => CliProvider::Claude,
        "codex" => CliProvider::Codex,
        _ => std::process::exit(2),
    };
    let runner = RealRunner::new(RunnerLaunchConfig {
        provider,
        cli_sha256: args[2].clone(),
        isolation: LinuxIsolation {
            runtime_root: args[1].clone().into(),
            bubblewrap: "/usr/bin/bwrap".into(),
            systemd_run: "/usr/bin/systemd-run".into(),
            gateway_socket: args[5].clone().into(),
        },
        source_checkout: args[3].clone().into(),
        attempt_root: args[3].clone().into(),
        retained_root: args[4].clone().into(),
        capability_file: "/not-read-by-probe".into(),
    });
    match runner.probe_report() {
        Ok(status) => println!(
            "{}",
            serde_json::to_string(&status).expect("serialize fixed status")
        ),
        Err(_) => {
            println!("{{\"unavailable\":{{\"reason\":\"deployment probe unavailable\"}}}}");
            std::process::exit(1);
        }
    }
}
