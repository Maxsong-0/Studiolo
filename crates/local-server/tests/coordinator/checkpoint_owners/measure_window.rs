//! 量测：冻结窗口不随写者的编辑数涨（BUG-050）。不进门禁，要数字时手工跑，打出来的行以 `MEASURE` 开头：
//! `STUDIOLO_MEASURE_LEDGER=700000 STUDIOLO_MEASURE_EDITS=1500 cargo test --release -p studiolo-local-server --lib
//! measure_window -- --ignored --nocapture`。
use super::{
    producer::Stage,
    producer_catch_up::{Measured, fill, seal_once},
    *,
};
use studiolo_core::VaultPath;
use studiolo_vault_broker::{
    Mutation, MutationActor, MutationKind, MutationMetadata, VaultWriter as _,
};

/// 没有审计记录的重放条数：169 上 1500 来次编辑之外还有这么多条。
const PLAIN: usize = 100;
/// 每编辑这么多次就把回执落一遍，正文审计的待发队列才不会满。
const FLUSH_EVERY: usize = 20;
/// 账本每造这么多条报一次进度；造到一半被时限掐掉也留得下数。
const CHUNK: usize = 50_000;

fn from_env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// 在正在服务的那个写者上新建 `edits` 篇笔记（每篇一条意图、一条结果回执），再写 [`PLAIN`] 次不留审计的。
fn edit(stage: &Stage, edits: usize) {
    let mut owner = stage.assembled.workbench.core.owner.lock().unwrap();
    let broker = owner.as_mut().unwrap();
    let note = |label: &str, index: usize| VaultPath::parse(format!("{label}-{index}.md")).unwrap();
    for index in 0..edits {
        broker
            .mutate_recorded(
                Mutation::create(
                    note("edited", index),
                    format!("第 {index} 篇").into_bytes(),
                    format!("measure-edit-{index}"),
                ),
                MutationMetadata::new(MutationActor::Obsidian, MutationKind::Replace).unwrap(),
            )
            .unwrap();
        if index % FLUSH_EVERY == FLUSH_EVERY - 1 || index + 1 == edits {
            flush_content(broker, &stage.assembled.human, 0);
        }
    }
    for index in 0..PLAIN {
        broker
            .mutate(Mutation::create(
                note("plain", index),
                b"plain".to_vec(),
                format!("measure-plain-{index}"),
            ))
            .unwrap();
    }
}

fn report(label: &str, measured: &Measured) {
    println!(
        "MEASURE {label} window_ms={} steps=[{}] after_ms={} steps=[{}]",
        measured.window.as_millis(),
        measured.window_steps.render(),
        measured.after.as_millis(),
        measured.after_steps.render()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "量测：造一本大账本要很久，只在要数字时手工跑"]
async fn measure_window_before_and_after_many_edits() {
    let ledger = from_env("STUDIOLO_MEASURE_LEDGER", 700_000);
    let edits = from_env("STUDIOLO_MEASURE_EDITS", 1_500);
    let budget = Duration::from_secs(
        u64::try_from(from_env("STUDIOLO_MEASURE_BUDGET_SECONDS", 1_200)).unwrap(),
    );
    let stage = Stage::new(true).await;
    let started = Instant::now();
    for chunk in 0..ledger.div_ceil(CHUNK) {
        // 造账本最费时，到点就不再造，按已有的条数量。
        if started.elapsed() > budget {
            println!("MEASURE fill stopped reason=budget");
            break;
        }
        let last = fill(
            &stage,
            &format!("bulk{chunk}-"),
            CHUNK.min(ledger - chunk * CHUNK),
        );
        println!(
            "MEASURE fill last_offset={last} seconds={:.1}",
            started.elapsed().as_secs_f64()
        );
    }
    // 第一次封存会把启动时欠的 WAL、缓存都热一遍，不算数。
    report("warm", &seal_once(&stage, 1));
    let before = seal_once(&stage, 2);
    report("edits=0", &before);
    edit(&stage, edits);
    report("edits=after-warm", &seal_once(&stage, 3));
    let after = seal_once(&stage, 4);
    report(&format!("edits={edits} plain={PLAIN}"), &after);
    println!(
        "MEASURE window_growth_ms={}",
        i128::try_from(after.window.as_millis()).unwrap()
            - i128::try_from(before.window.as_millis()).unwrap()
    );
    stage.assembled.workbench.shutdown().await.unwrap();
}
