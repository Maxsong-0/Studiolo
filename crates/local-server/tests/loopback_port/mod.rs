//! 本机端口交接（T-171）：夹具「只绑定不监听」地持有端口，直到子进程退出才释放；子进程照常以
//! `--port` 绑定并监听同一端口。就绪只认子进程自己打印的启动行，不认「能连上」。
//!
//! 旧写法先 `bind(0)` 取号、立即释放再交给子进程：窗口内同机任何进程的自动分配都能拿走这个端口，
//! 「能连上」于是可能连到别人的服务，对方一关监听，排队的连接就收到 RST（CI 上的 ConnectionReset）。
//!
//! 依赖的 Linux 语义（两台 CI 机实测）：
//! - 占位带 `SO_REUSEADDR` 且不监听时，同样带该选项的显式绑定可以共享端口（socket(7)）；产品经 tokio
//!   绑定、std 的监听都带这个选项，所以子进程与负向用例里的占用者都绑得上；
//! - `net.ipv4.ip_autobind_reuse=0`（默认）时，别的进程 `bind(0)`／`connect` 的自动分配不会选中已有
//!   绑定的端口，占位期间端口不会被同机进程拿走。
//!
//! BSD／macOS 上同一地址的二次绑定要求双方都设 `SO_REUSEPORT`，产品不设，因此这些进程测试只在 Linux 成立。

use std::{
    io::{BufRead, BufReader, Read},
    net::{Ipv4Addr, SocketAddr},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::Duration,
};

/// 本机只读产品的启动行前缀；产品在绑定成功之后才打印它。
pub fn local_server_announcement(address: SocketAddr) -> String {
    format!("Studiolo：http://{address}（")
}

/// 已宣布就绪的产品子进程：端口占位与子进程同寿；Drop 时收走子进程，测试失败时打印它的去向。
pub struct Announced {
    pub child: Child,
    pub address: SocketAddr,
    _reservation: Reservation,
}

impl Announced {
    /// `command` 须已带上 `--port <reservation 的端口>`；这里接管标准输出与标准错误后启动并等启动行。
    pub fn spawn(mut command: Command, reservation: Reservation, budget: Duration) -> Self {
        let address = reservation.address;
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut process = Self {
            child,
            address,
            _reservation: reservation,
        };
        let announcement = local_server_announcement(address);
        if let Err(reason) = wait_announced(&mut process.child, &announcement, budget) {
            panic!("测试子进程未就绪：{reason}");
        }
        process
    }
}

impl Drop for Announced {
    fn drop(&mut self) {
        let report = reap(&mut self.child);
        if thread::panicking() {
            eprintln!("{report}");
        }
    }
}

/// 端口占位：必须活到子进程退出（或被收走）之后。
pub struct Reservation {
    _socket: tokio::net::TcpSocket,
    pub address: SocketAddr,
}

impl Reservation {
    pub fn new() -> Self {
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.set_reuseaddr(true).unwrap();
        socket.bind((Ipv4Addr::LOCALHOST, 0).into()).unwrap();
        let address = socket.local_addr().unwrap();
        Self {
            _socket: socket,
            address,
        }
    }
}

/// 读子进程标准输出，直到出现以 `announcement` 开头的一行；之后继续排空，子进程不会因管道写满而阻塞。
/// 子进程先关闭标准输出（通常是已经退出）或超出预算都返回说明，并收走子进程、带上退出状态与标准错误。
pub fn wait_announced(
    child: &mut Child,
    announcement: &str,
    budget: Duration,
) -> Result<(), String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "子进程标准输出未接管".to_owned())?;
    let (sender, announced) = mpsc::channel();
    let expected = announcement.as_bytes().to_vec();
    thread::spawn(move || {
        let mut sender = Some(sender);
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();
        while reader
            .read_until(b'\n', &mut line)
            .is_ok_and(|read| read > 0)
        {
            if line.starts_with(&expected)
                && let Some(sender) = sender.take()
            {
                let _ = sender.send(());
            }
            line.clear();
        }
    });
    match announced.recv_timeout(budget) {
        Ok(()) => Ok(()),
        Err(RecvTimeoutError::Disconnected) => Err(format!(
            "子进程没有宣布「{announcement}」就关闭了标准输出；{}",
            reap(child)
        )),
        Err(RecvTimeoutError::Timeout) => Err(format!(
            "子进程 {} 秒内没有宣布「{announcement}」；{}",
            budget.as_secs(),
            reap(child)
        )),
    }
}

/// 收走子进程并说明它的去向：先前是否已自行退出、退出状态、标准错误（有界）。
/// 用于区分「端口被抢／连错对象」与「子进程自己退出」，测试失败时由夹具打印。
pub fn reap(child: &mut Child) -> String {
    let exited_before = child.try_wait().ok().flatten().is_some();
    let _ = child.kill();
    let status = child.wait().ok();
    let mut stderr = Vec::new();
    if let Some(pipe) = child.stderr.take() {
        let _ = pipe.take(8 * 1024).read_to_end(&mut stderr);
    }
    format!(
        "子进程{}，退出状态 {status:?}；标准错误：{}",
        if exited_before {
            "已先行退出"
        } else {
            "仍在运行，已由夹具终止"
        },
        String::from_utf8_lossy(&stderr).trim()
    )
}
