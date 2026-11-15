//! 整读校验合流的规则（T-211）：受理之后才开始、并且成功的最近一次校验才共用；
//! 失败不外传，也不退回去用失败之前的旧结果；一个接一个的请求仍然一人一次。
use super::SharedScan;
use crate::verifier::Verifier;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
        mpsc,
    },
};

/// 每做一次真实校验就加一，返回「这是第几次」。
fn counted(count: &AtomicU32) -> Result<u32, String> {
    Ok(count.fetch_add(1, Ordering::SeqCst) + 1)
}

#[test]
fn requests_one_after_another_each_get_their_own_scan() {
    let scans = SharedScan::new();
    let count = AtomicU32::new(0);
    for expected in 1..=3 {
        let admitted = scans.admit();
        assert_eq!(scans.covered_or(admitted, || counted(&count)), Ok(expected));
    }
    assert_eq!(count.load(Ordering::SeqCst), 3, "先后到达的请求不合流");
}

#[test]
fn only_a_scan_that_began_after_admission_is_shared() {
    let scans = SharedScan::new();
    let count = AtomicU32::new(0);
    let early = scans.admit();
    // 后台重扫在 early 受理之后才开始：它的全部读取都晚于 early。
    assert_eq!(scans.run(|| counted(&count)), Ok(1));
    let late = scans.admit();
    assert_eq!(scans.covered_or(early, || counted(&count)), Ok(1));
    assert_eq!(count.load(Ordering::SeqCst), 1, "受理在前的请求共用这一次");
    assert_eq!(scans.covered_or(late, || counted(&count)), Ok(2));
    assert_eq!(
        count.load(Ordering::SeqCst),
        2,
        "受理在后的请求必须自己再校验一次"
    );
}

#[test]
fn a_failed_scan_is_never_shared_and_hides_the_success_before_it() {
    let scans = SharedScan::new();
    let count = AtomicU32::new(0);
    let early = scans.admit();
    assert_eq!(scans.run(|| counted(&count)), Ok(1));
    // 之后的一次校验发现副本坏了：early 不能再拿第 1 次的旧结果。
    assert_eq!(
        scans.run(|| Err::<u32, _>("副本漂移".to_owned())),
        Err("副本漂移".to_owned())
    );
    assert_eq!(scans.covered_or(early, || counted(&count)), Ok(2));
    // 失败本身也不外传：下一个请求自己校验，拿到自己的结论。
    let waiting = scans.admit();
    assert_eq!(
        scans.run(|| Err::<u32, _>("副本漂移".to_owned())),
        Err("副本漂移".to_owned())
    );
    assert_eq!(scans.covered_or(waiting, || counted(&count)), Ok(3));
}

#[test]
fn a_panicking_scan_leaves_no_stale_result() {
    let scans = SharedScan::new();
    let count = AtomicU32::new(0);
    let early = scans.admit();
    assert_eq!(scans.run(|| counted(&count)), Ok(1));
    let crashed = catch_unwind(AssertUnwindSafe(|| {
        scans.run(|| -> Result<u32, String> { panic!("校验中途崩溃") })
    }));
    assert!(crashed.is_err());
    assert_eq!(scans.covered_or(early, || counted(&count)), Ok(2));
    assert_eq!(count.load(Ordering::SeqCst), 2, "崩溃之前的旧结果不能再用");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sixteen_concurrent_requests_cost_two_scans_on_the_dedicated_thread() {
    let verifier = Arc::new(Verifier::start().expect("起专用线程"));
    let scans = Arc::new(SharedScan::new());
    let count = Arc::new(AtomicU32::new(0));
    let (began, first_began) = tokio::sync::oneshot::channel();
    let (release, gate) = mpsc::channel::<()>();

    // 第 1 个请求的校验在途，卡住不放。
    let first = tokio::spawn({
        let (verifier, scans, count) = (verifier.clone(), scans.clone(), count.clone());
        let admitted = scans.admit();
        async move {
            verifier
                .run(move || {
                    scans.covered_or(admitted, || {
                        let _ = began.send(());
                        let _ = gate.recv();
                        counted(&count)
                    })
                })
                .await
        }
    });
    first_began.await.expect("第 1 次校验开始");

    // 其余 15 个在第 1 次校验途中到达：第 1 次开始得比它们早，不能给它们用。
    let mut rest = tokio::task::JoinSet::new();
    for _ in 0..15 {
        let (verifier, scans, count) = (verifier.clone(), scans.clone(), count.clone());
        let admitted = scans.admit();
        rest.spawn(async move {
            verifier
                .run(move || scans.covered_or(admitted, || counted(&count)))
                .await
        });
    }
    release.send(()).expect("放开第 1 次校验");

    assert_eq!(first.await.expect("第 1 个请求"), Ok(Ok(1)));
    while let Some(done) = rest.join_next().await {
        assert_eq!(
            done.expect("并发请求"),
            Ok(Ok(2)),
            "其余请求共用受理之后的那一次"
        );
    }
    assert_eq!(count.load(Ordering::SeqCst), 2, "16 路并发只做两次整读");
}
