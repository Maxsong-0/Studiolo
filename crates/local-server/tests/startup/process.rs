//! 仅管理本测试派生的产品进程；清空继承环境，有界等待，退出后核对监听状态。

use std::{
    ffi::OsString,
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

pub struct Running {
    child: Option<Child>,
    pub address: SocketAddr,
}

impl Running {
    pub fn start(root: &Path, args: Vec<String>) -> Self {
        Self::start_os(root, args.into_iter().map(OsString::from).collect())
    }

    pub fn start_os(root: &Path, args: Vec<OsString>) -> Self {
        Self::launch(
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
            root,
            args.into_iter().map(OsString::from).collect(),
            false,
            environment,
        )
    }

    fn launch(
        root: &Path,
        args: Vec<OsString>,
        tcp_port: bool,
        environment: &[(&str, &str)],
    ) -> Self {
        let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = reservation.local_addr().unwrap();
        drop(reservation);
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
        }
    }

    pub fn wait(mut self) -> Output {
        let deadline = Instant::now() + Duration::from_secs(8);
        while self.child.as_mut().unwrap().try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "产品进程未在启动失败预算内退出");
            thread::sleep(Duration::from_millis(20));
        }
        self.child.take().unwrap().wait_with_output().unwrap()
    }

    pub fn wait_ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            assert!(
                self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "旧本机模式意外退出"
            );
            if TcpStream::connect_timeout(&self.address, Duration::from_millis(100)).is_ok() {
                return;
            }
            assert!(Instant::now() < deadline, "旧本机模式未及时监听");
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
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
    let running = Running::start_os(root, args);
    let address = running.address;
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
    assert!(
        TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err(),
        "失败进程退出后不得留下监听"
    );
}
