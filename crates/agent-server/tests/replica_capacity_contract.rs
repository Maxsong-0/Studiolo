//! 托管副本在 Markdown 合计上限时的并发读时延与常驻内存（T-215，vault BUG-039）。
//!
//! agent 每次内容请求都整读校验全部 Markdown，每个请求拿到的都是它到达之后重新完整校验过的代次。
//! 上限能定多大，看满额副本在 16 路并发下（`http_text.rs` 的 `Semaphore::new(16)`，同 T-185 探针）
//! 还能不能按时答完、常驻内存离单元 `MemoryMax=512M` 还有多远。本测试按 `NOTES_TOTAL_LIMIT` 生成满额副本：
//! 篇幅照真实库 2026-11-18 的分布（平均约 11 KiB、中位约 6 KiB），wikilink 与标题密度也照真实库，正文全是虚构的；
//! 全部笔记放在一个目录里（同 T-185 探针，也是逐目录开销最重的形状：原先逐篇按路径重读时，耗时随同目录篇数平方增长）。
//!
//! 子进程像生产一样先 `mlockall`、开索引缓存，打 2 轮 16 路并发读；再像 Mac 每 60 秒推来小改动那样连换
//! [`CHANGES`] 个各改了几十篇的新代次（每次都触发重新校验加增量重建读模型），每换一次再打 2 轮。mlockall 之后
//! 堆顶只升不降，连换几次才看得出常驻内存会不会一路往上爬。最慢的一个请求不超过 [`LATENCY_BUDGET_MS`]、
//! 常驻匿名内存不超过 [`MEMORY_BUDGET_KIB`]。上限往上调、或者校验又变成随篇数超线性增长，它先红。
//!
//! 只在 release 构建里跑：debug 构建的解析与建模慢一个数量级，量不出生产时延。ci 套的 `replica-capacity`
//! 一步用 `cargo --locked test --release -p studiolo-agent-server --test replica_capacity_contract` 跑它。
#![cfg(target_os = "linux")]

#[path = "../../supply/tests/replica_support/mod.rs"]
mod replica_support;
#[allow(dead_code)]
#[path = "http_text/support.rs"]
mod support;

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

use replica_support::{ReplicaFixture, private_dir};
use studiolo_agent_server::options::ServeOptions;
use studiolo_supply::replica::NOTES_TOTAL_LIMIT;

/// 置位后本测试二进制作为探针运行；产品源码没有这个入口。
const PROBE: &str = "STUDIOLO_REPLICA_CAPACITY_PROBE";
/// 父进程把准备好的路径经这几个环境变量交给探针。
const ROOT: &str = "STUDIOLO_REPLICA_CAPACITY_ROOT";
/// 目录里放着依次要换上的控制文件 `next-1.json`…`next-<CHANGES>.json`。
const NEXT_STATES: &str = "STUDIOLO_REPLICA_CAPACITY_NEXT_STATES";
const DATA: &str = "STUDIOLO_REPLICA_CAPACITY_DATA";
const INDEX: &str = "STUDIOLO_REPLICA_CAPACITY_INDEX";

/// 与 HTTP 入口的并发供给上限一致（同 T-185 探针）。
const CONCURRENCY: usize = 16;
/// 每个代次上打几轮。
const BURSTS: usize = 2;
/// 首个满额代次之后再换几次代次。
const CHANGES: usize = 3;
/// 最慢一个请求的时延上限：测试客户端超时（`http_text/support.rs` 的 20 秒）的一半，留出慢一倍的主机的余量。
const LATENCY_BUDGET_MS: u128 = 10_000;
/// 常驻匿名内存上限：单元 `MemoryMax=512M` 的四分之三。mlockall 之后释放的页退不回去，
/// 读数就是进程起来以来的最高水位；剩下的四分之一留给锁定的程序文件页与连接突发。
const MEMORY_BUDGET_KIB: u64 = 384 * 1024;

/// 换代次时改动的篇数间隔：第 k 次换代改编号除以它余 k 的那些篇（满额约四十来篇），同大小改字节，合计不变。
const CHANGE_EVERY: usize = 97;

/// 真实库 2026-11-18 白名单 Markdown 篇幅（1932 篇）按 40 档等频分位取的档内均值，单位字节。只有数字。
const SIZES: [usize; 40] = [
    393, 1188, 1482, 1670, 1847, 2046, 2211, 2415, 2585, 2793, 3034, 3241, 3442, 3741, 4061, 4313,
    4559, 4858, 5184, 5590, 6020, 6449, 6995, 7537, 8159, 8812, 9525, 10372, 11403, 12541, 13810,
    15514, 17278, 18855, 21182, 23811, 27052, 31490, 37419, 99975,
];
/// 真实库约每 11 行一个标题、每 18 行一条 wikilink（每 KiB 0.76 个标题、0.47 条链接，平均行长 118 字节）。
const HEADING_EVERY: usize = 11;
const LINK_EVERY: usize = 18;

const INDEX_NOTE: &str =
    "# Project\n\n| 项 | 当前值 |\n|---|---|\n| 当前版本 | capacity |\n| 状态 | capacity |\n";

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "满额副本的时延只在 release 构建里有意义：ci 套 replica-capacity 一步用 --release 跑"
)]
fn notes_limit_replica_answers_concurrent_reads_within_budget() {
    let corpus = Corpus::publish();
    let mut probe = Command::new(std::env::current_exe().expect("取得测试二进制路径"));
    probe
        .args([
            "--exact",
            "replica_capacity_probe",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(PROBE, "1")
        .env(ROOT, &corpus.fixture.root)
        .env(NEXT_STATES, &corpus.next_states)
        .env(DATA, corpus.fixture.temp.path().join("agent-data"))
        .env(INDEX, &corpus.index);
    // glibc 的 malloc 调参会改变 arena 行为，探针一律不继承（同 T-185 内存探针）。
    for (key, _) in std::env::vars_os() {
        if key
            .to_str()
            .is_some_and(|key| key.starts_with("MALLOC_") || key == "GLIBC_TUNABLES")
        {
            probe.env_remove(key);
        }
    }
    let output = probe.output().expect("启动探针子进程");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "探针失败：\n{stdout}\n{stderr}");
    // libtest 会把探针的第一行输出接在「test replica_capacity_probe ... 」后面，所以按「键=」切分。
    let reading = |key: &str| -> u64 {
        stdout
            .lines()
            .find_map(|line| line.split_once(&format!("{key}=")))
            .and_then(|(_, value)| value.trim().parse().ok())
            .unwrap_or_else(|| panic!("探针没有报告 {key}：\n{stdout}\n{stderr}"))
    };
    let slowest = u128::from(reading("SLOWEST_MS"));
    let rss_anon = reading("RSS_ANON_KIB");
    let trajectory = stdout
        .lines()
        .find_map(|line| line.split_once("RSS_TRAJECTORY_KIB="))
        .map_or("", |(_, value)| value.trim());
    eprintln!(
        "副本容量自证：上限 {} KiB，满额 {} 篇 {} KiB；{} 个代次各 {BURSTS} 轮 {CONCURRENCY} 路并发读，\
         最慢 {slowest} ms（上限 {LATENCY_BUDGET_MS} ms）；常驻匿名内存 {rss_anon} KiB（上限 {MEMORY_BUDGET_KIB} KiB，\
         每个代次打完时 {trajectory}），锁定程序文件页 {} KiB",
        NOTES_TOTAL_LIMIT / 1024,
        corpus.notes,
        corpus.bytes / 1024,
        CHANGES + 1,
        reading("RSS_FILE_KIB"),
    );
    assert!(
        slowest <= LATENCY_BUDGET_MS,
        "满额副本 {CONCURRENCY} 路并发读最慢 {slowest} ms，超过 {LATENCY_BUDGET_MS} ms：\
         整读校验或读模型重建变慢了（篇数多时是否又成了超线性），或者上限调得超出了校验能扛的量"
    );
    assert!(
        rss_anon <= MEMORY_BUDGET_KIB,
        "满额副本让常驻匿名内存到了 {rss_anon} KiB，超过 {MEMORY_BUDGET_KIB} KiB（单元 MemoryMax=512M）：\
         上限调高了就得先压内存"
    );
}

/// 探针：在一次性子进程里与生产一样加固，再起真实 HTTP 服务、打并发读请求并报告读数。
#[test]
fn replica_capacity_probe() {
    if std::env::var_os(PROBE).is_none() {
        return;
    }
    let residency = studiolo_process_safety::harden().unwrap_or_else(|refusal| {
        panic!(
            "与生产一样锁定地址空间失败（memlock 上限不足时按设计拒绝，不得据此跳过）：{refusal}"
        )
    });
    eprintln!("秘密驻留面：{residency}");
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("建立运行时")
        .block_on(probe());
}

async fn probe() {
    let path = |key: &str| PathBuf::from(std::env::var_os(key).unwrap_or_else(|| panic!("{key}")));
    let root = path(ROOT);
    let server = support::spawn(ServeOptions {
        board_socket: None,
        index_cache: Some(path(INDEX)),
        vault: None,
        replica_root: Some(root.clone()),
        replica_max_age_secs: 300,
        allow: vec!["project".into()],
        data: path(DATA),
        policy: None,
        port: 0,
    })
    .await;
    let (session, _, signpost) = server.handshake().await;
    // 满额副本早过了提醒线：路牌上必须有容量提醒（vault BUG-039 的提前量）。
    assert!(
        signpost.contains("⚠ 副本 Markdown 合计 "),
        "满额副本的路牌没有容量提醒：\n{signpost}"
    );
    let (status, _) = timed(server.get(&route(0), Some(&session)))
        .await
        .expect("预热读请求");
    assert_eq!(status, 200);
    let mut slowest = Duration::ZERO;
    let mut trajectory = Vec::new();
    for change in 0..=CHANGES {
        if change > 0 {
            // 换上下一代次：同接收端一样先写好再原子替换控制文件。
            let staged = root.join("control/state-next.json");
            fs::copy(
                path(NEXT_STATES).join(format!("next-{change}.json")),
                &staged,
            )
            .expect("暂存下一代次的控制文件");
            fs::set_permissions(&staged, fs::Permissions::from_mode(0o600)).expect("收紧权限");
            fs::rename(&staged, root.join("control/state.json")).expect("换上下一代次");
        }
        for _ in 0..BURSTS {
            slowest = slowest.max(burst(&server, &session).await);
        }
        trajectory.push(status_kib("RssAnon:").to_string());
    }
    println!("SLOWEST_MS={}", slowest.as_millis());
    println!("RSS_TRAJECTORY_KIB={}", trajectory.join(","));
    println!("RSS_ANON_KIB={}", status_kib("RssAnon:"));
    println!("RSS_FILE_KIB={}", status_kib("RssFile:"));
}

/// 本进程 `/proc/self/status` 里的一项（KiB）。锁定后释放的堆页仍算在 `RssAnon` 里，cgroup 记账看的也是它。
fn status_kib(name: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .expect("读取 /proc/self/status")
        .lines()
        .find_map(|line| line.strip_prefix(name))
        .and_then(|value| value.split_whitespace().next()?.parse().ok())
        .unwrap_or_else(|| panic!("/proc/self/status 里有 {name}"))
}

/// 16 路同时读不同的笔记，全部要 200；返回最慢那一个的时延。
async fn burst(server: &support::HttpServer, session: &str) -> Duration {
    let mut requests = tokio::task::JoinSet::new();
    for note in 0..CONCURRENCY {
        requests.spawn(timed(server.get(&route(note), Some(session))));
    }
    let mut slowest = Duration::ZERO;
    while let Some(done) = requests.join_next().await {
        let (status, took) = done
            .expect("并发读任务")
            .expect("并发读请求（测试客户端 20 秒超时）");
        assert_eq!(status, 200);
        slowest = slowest.max(took);
    }
    slowest
}

/// 发请求并读完响应体（像真实客户端一样让服务端把这次供给完整走完），返回状态码与时延。
async fn timed(
    request: reqwest::RequestBuilder,
) -> reqwest::Result<(reqwest::StatusCode, Duration)> {
    let start = Instant::now();
    let response = request.send().await?;
    let status = response.status();
    response.bytes().await?;
    Ok((status, start.elapsed()))
}

fn route(note: usize) -> String {
    format!("/read/note?path=project%2Fnotes%2Fn{note:05}.md")
}

/// 1 + [`CHANGES`] 个满额代次：先倒着发后面那几个、依次记下它们的控制文件，最后发要先供给的那个
/// （控制文件指向它）。来源时间逐代加一秒，回执不回退。
struct Corpus {
    fixture: ReplicaFixture,
    next_states: PathBuf,
    index: PathBuf,
    notes: usize,
    bytes: usize,
}

impl Corpus {
    fn publish() -> Self {
        let fixture = ReplicaFixture::new();
        let budget =
            usize::try_from(NOTES_TOTAL_LIMIT).expect("上限放得进 usize") - INDEX_NOTE.len();
        let mut sizes = Vec::new();
        let mut bytes = 0;
        loop {
            let size =
                note_body(sizes.len(), SIZES[(sizes.len() * 17) % SIZES.len()], 'A', 1).len();
            if bytes + size > budget {
                break;
            }
            bytes += size;
            sizes.push(SIZES[(sizes.len() * 17) % SIZES.len()]);
        }
        let notes = sizes.len();
        // 篇幅档里最大的约 98 KiB，凑到差一篇放不下为止，满额在上限的 256 KiB 以内。
        assert!(bytes + 256 * 1024 > budget, "没凑到满额");
        // 第 k 个代次里，编号除以 CHANGE_EVERY 余 1..=k 的篇都是 B 版：每换一次正好多改一组。
        let generation = |changes: usize| -> Vec<(String, Vec<u8>)> {
            (0..notes)
                .map(|note| {
                    let version = if (1..=changes).contains(&(note % CHANGE_EVERY)) {
                        'B'
                    } else {
                        'A'
                    };
                    (
                        format!("project/notes/n{note:05}.md"),
                        note_body(note, sizes[note], version, notes),
                    )
                })
                .collect()
        };
        let publish = |generation: &str, observed: i64, bodies: &[(String, Vec<u8>)]| {
            let mut entries: Vec<(&str, Option<&[u8]>)> = vec![
                ("project/00-总览", None),
                ("project/00-总览/INDEX.md", Some(INDEX_NOTE.as_bytes())),
                ("project/notes", None),
            ];
            entries.extend(
                bodies
                    .iter()
                    .map(|(path, body)| (path.as_str(), Some(body.as_slice()))),
            );
            fixture.publish_entries(generation, observed, &entries);
        };
        let next_states = fixture.temp.path().join("next-states");
        fs::create_dir(&next_states).expect("建控制文件暂存目录");
        let id = |change: usize| format!("{:032x}", change + 2);
        for change in (1..=CHANGES).rev() {
            let observed = fixture.observed + 1000 * i64::try_from(change).expect("代次数很小");
            publish(&id(change), observed, &generation(change));
            fs::copy(
                fixture.root.join("control/state.json"),
                next_states.join(format!("next-{change}.json")),
            )
            .expect("记下下一代次");
        }
        publish(&id(0), fixture.observed, &generation(0));
        let index = fixture.temp.path().join("index-cache");
        private_dir(&index);
        Self {
            fixture,
            next_states,
            index,
            notes,
            bytes: bytes + INDEX_NOTE.len(),
        }
    }
}

/// 一篇虚构笔记：标题行带版本号（`A`/`B` 同字节数，换代次只改字节、不改大小），之后按真实库的
/// 行长、标题与 wikilink 密度凑到约 `size` 字节。不含任何真实内容、token 或口令。
fn note_body(note: usize, size: usize, version: char, notes: usize) -> Vec<u8> {
    let mut body = format!("# 虚构笔记 {note:05} 版本 {version}\n\n");
    let mut line = 0;
    while body.len() < size {
        if line % HEADING_EVERY == HEADING_EVERY - 1 {
            body.push_str(&format!("\n## 虚构小节 {line}\n\n"));
        } else if line % LINK_EVERY == LINK_EVERY - 1 {
            let target = (note * 7919 + line * 104_729) % notes.max(1);
            body.push_str(&format!(
                "- 第 {line} 行另见 [[n{target:05}]]，虚构的交叉引用，没有任何真实信息。\n"
            ));
        } else {
            body.push_str(&format!(
                "- 第 {line} 行，虚构词 w{note}x{line}，用来凑出接近真实笔记的行长，不是秘密。\n"
            ));
        }
        line += 1;
    }
    body.into_bytes()
}
