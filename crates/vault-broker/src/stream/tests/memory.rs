use super::*;
#[test]
fn full_large_pipeline_is_measured_in_an_isolated_process() {
    let _guard = LARGE.enter();
    let executable = std::env::current_exe().unwrap();
    let mut command = std::process::Command::new("/usr/bin/time");
    #[cfg(target_os = "macos")]
    command.arg("-l");
    #[cfg(target_os = "linux")]
    command.arg("-v");
    let output = command
        .arg(executable)
        .args([
            "--exact",
            "stream::tests::memory::large_child",
            "--ignored",
            "--test-threads=1",
            "--nocapture",
        ])
        .env("LC_ALL", "C")
        .env("STUDIOLO_PRIVATE_STREAM_MEMORY_CHILD", "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        stderr
    );
    #[cfg(target_os = "macos")]
    let peak = stderr
        .lines()
        .find(|line| line.contains("maximum resident set size"))
        .and_then(|line| line.split_whitespace().next())
        .unwrap()
        .parse::<u64>()
        .unwrap();
    #[cfg(target_os = "linux")]
    let peak = stderr
        .lines()
        .find(|line| line.contains("Maximum resident set size (kbytes)"))
        .and_then(|line| line.rsplit(':').next())
        .unwrap()
        .trim()
        .parse::<u64>()
        .unwrap()
        * 1024;
    // 单对象超过 200 MiB；进程峰值需低于 128 MiB，能抓住整对象 Vec 回归。
    assert!(peak < 128 * 1024 * 1024, "measured peak RSS: {peak}");
}
#[test]
#[ignore = "仅由受控父测试单独启动并测量峰值 RSS"]
fn large_child() {
    assert_eq!(
        std::env::var("STUDIOLO_PRIVATE_STREAM_MEMORY_CHILD").unwrap(),
        "1"
    );
    let f = Fixture::new();
    let mut b = f.open();
    let source = f.pattern(LARGE_BYTES);
    let request = request(&source, "memory", Precondition::MustNotExist);
    let ticket = b
        .prepare_stream_mutation(request.clone(), &source, limits(), metadata())
        .unwrap();
    let receipt = b.commit_stream_mutation(ticket).unwrap();
    assert_eq!(installed(&b).identity, request.content);
    b.mutate(Mutation::replace(
        path(),
        b"small".to_vec(),
        receipt.revision,
        "small",
    ))
    .unwrap();
    assert_eq!(
        crate::history::blob::selected(&b, &path(), receipt.revision).unwrap(),
        request.content
    );
}
