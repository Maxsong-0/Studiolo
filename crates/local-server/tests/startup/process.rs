//! 仅管理本测试派生的产品进程；清空继承环境，有界等待，退出后核对监听状态。
//! 端口由本夹具持有到子进程退出，就绪只认子进程自己的启动行（T-171，见 `loopback_port`）。

#[path = "../loopback_port/mod.rs"]
#[allow(dead_code)]
mod loopback_port;

pub use loopback_port::Reservation;

use std::{
    ffi::OsString,
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

pub struct Running {
    child: Option<Child>,
    pub address: SocketAddr,
    // 字段在 Drop::drop 收走子进程之后才释放：端口直到子进程退出都不交还系统。
    _reservation: Reservation,
}

impl Running {
    pub fn start(root: &Path, args: Vec<String>) -> Self {
        Self::start_on(Reservation::new(), root, args)
    }

    /// 在调用方给定的占位上启动旧本机模式；负向自检用它先在同一端口放一个占用者。
    pub fn start_on(reservation: Reservation, root: &Path, args: Vec<String>) -> Self {
        Self::start_os_on(
            reservation,
            root,
            args.into_iter().map(OsString::from).collect(),
        )
    }

    pub fn start_os(root: &Path, args: Vec<OsString>) -> Self {
        Self::start_os_on(Reservation::new(), root, args)
    }

    fn start_os_on(reservation: Reservation, root: &Path, args: Vec<OsString>) -> Self {
        Self::launch(
            reservation,
            root,
            args,
            true,
            &[(
                "STUDIOLO_TOTP_SEED_HEX",
                "invalid-environment-secret-canary",
            )],
        )
    }

    pub fn start_factor(root: &Path, args: Vec<String>, environment: &[(&str, &str)]) -> Self {
        Self::launch(
            Reservation::new(),
            root,
            args.into_iter().map(OsString::from).collect(),
            false,
            environment,
        )
    }

    fn launch(
        reservation: Reservation,
        root: &Path,
        args: Vec<OsString>,
        tcp_port: bool,
        environment: &[(&str, &str)],
    ) -> Self {
        let address = reservation.address;
        // 仅测试运行器可选择已构建的生产二进制；该配置不传入产品环境。
        let binary = std::env::var_os("STUDIOLO_TEST_SERVER_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_studiolo-local-server")));
        assert!(
            binary.is_absolute() && binary.is_file(),
            "测试产品二进制必须为已存在的绝对文件路径"
        );
        let mut command = Command::new(binary);
        command.args(args);
        if tcp_port {
            command.arg("--port").arg(address.port().to_string());
        }
        let child = command
            .current_dir(root)
            .env_clear()
            // 普通模式注入无效canary验证不读取；因子负测只使用显式虚构材料。
            .envs(environment.iter().copied())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        Self {
            child: Some(child),
            address,
            _reservation: reservation,
        }
    }

    /// 等子进程自行退出；占位随 `Running` 一起活到调用方丢弃它，退出后的端口核对不受他人干扰。
    pub fn wait(&mut self) -> Output {
        let deadline = Instant::now() + Duration::from_secs(8);
        while self.child.as_mut().unwrap().try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "产品进程未在启动失败预算内退出");
            thread::sleep(Duration::from_millis(20));
        }
        self.child.take().unwrap().wait_with_output().unwrap()
    }

    pub fn wait_ready(&mut self) {
        if let Err(reason) = self.ready() {
            panic!("旧本机模式未就绪：{reason}");
        }
    }

    /// 就绪＝子进程自己宣布已在占位端口上监听；端口上有别人的监听也不算。
    pub fn ready(&mut self) -> Result<(), String> {
        let announcement = loopback_port::local_server_announcement(self.address);
        loopback_port::wait_announced(
            self.child.as_mut().unwrap(),
            &announcement,
            Duration::from_secs(8),
        )
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // 测试失败时带出子进程去向，区分「连错了对象」与「子进程自己退出」。
            let report = loopback_port::reap(&mut child);
            if thread::panicking() {
                eprintln!("{report}");
            }
        }
    }
}

pub fn reject(root: &Path, args: Vec<String>, forbidden: &[&str]) {
    reject_os(
        root,
        args.into_iter().map(OsString::from).collect(),
        forbidden,
    );
}

pub fn reject_os(root: &Path, args: Vec<OsString>, forbidden: &[&str]) {
    let mut running = Running::start_os(root, args);
    let output = running.wait();
    assert!(!output.status.success(), "不合格装配不能成功退出");
    assert!(output.stdout.is_empty(), "启动失败不能宣布已监听或输出配置");
    assert!(
        !output.stderr.is_empty() && output.stderr.len() <= 4096,
        "失败说明必须非空且有界"
    );
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        !error.contains("panicked") && !error.contains("RUST_BACKTRACE"),
        "启动错误必须正常返回，不经默认 panic 输出原始参数"
    );
    assert!(
        forbidden.iter().all(|value| !error.contains(value)),
        "失败说明不得泄漏夹具配置或环境内容"
    );
    // 这里只证明退出后的端口状态；不声称观测到了全过程零监听或零网络数据包。
    // 端口仍由 running 的占位持有，别的进程拿不到，连得上只能是本进程树留下的监听。
    assert!(
        TcpStream::connect_timeout(&running.address, Duration::from_millis(100)).is_err(),
        "失败进程退出后不得留下监听"
    );
}
