//! 假 launcher：临时目录里同名 `launcher.sock` 上按协议应答的线程，加 `client::call` 的一组
//! 契约用例；`verify_backend_at` 的一组在 `launcher_fake/verify.rs`。
//!
//! 它只复现线上契约（帧、闭集、`Verdict` 映射、session 状态机的可见行为），不建 unit、
//! 不读 cgroupfs；所以任何用例都不能据此声称平台受支持——真实 systemd／cgroup／隔离契约
//! 只能在管理员置备后的隔离 Linux 上实测（平台契约未验，见 `platform.rs` 的忽略用例）。

mod script;
mod verify;

pub use script::{Probe, Script, Tamper};

use super::support::*;
use crate::linux::ipc::{self, ALLOWED_KEYS, FRAME_MAX_BYTES};
use crate::linux::protocol::{Empty, LaunchFrame, Request, Response, Verdict};
use crate::linux::service::{argv_digest, build_argv};
use crate::linux::{ContainmentGap, client};
use crate::owned_run::{RunId, RunOperation};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// 临时目录里的假 launcher；`Drop` 时叫醒并回收应答线程。
pub struct FakeLauncher {
    directory: tempfile::TempDir,
    socket: PathBuf,
    script: Arc<Mutex<Script>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeLauncher {
    pub fn start() -> Self {
        Self::with(Script::default())
    }

    pub fn with(script: Script) -> Self {
        let directory = tempfile::tempdir().expect("临时目录可建");
        let socket = directory.path().join("launcher.sock");
        let listener = UnixListener::bind(&socket).expect("临时 socket 可绑定");
        let script = Arc::new(Mutex::new(script));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (script, stop) = (Arc::clone(&script), Arc::clone(&stop));
            std::thread::spawn(move || serve(&listener, &script, &stop))
        };
        Self {
            directory,
            socket,
            script,
            stop,
            thread: Some(thread),
        }
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub fn directory(&self) -> &Path {
        self.directory.path()
    }

    pub fn script(&self) -> MutexGuard<'_, Script> {
        self.script.lock().expect("脚本未中毒")
    }

    pub fn call(&self, request: &Request) -> Result<Response, ContainmentGap> {
        client::call(&self.socket, request)
    }
}

impl Drop for FakeLauncher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // 自己连一次把阻塞在 accept 的线程叫醒；它看到停止标记就退出。
        let _ = UnixStream::connect(&self.socket);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(listener: &UnixListener, script: &Mutex<Script>, stop: &AtomicBool) {
    for connection in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let Ok(mut stream) = connection else { break };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        if let Some(reply) = handle(&mut stream, script) {
            let _ = stream.write_all(&reply);
        }
    }
}

/// 读一帧请求：帧、闭集、形状任一不合即 `refused{protocol}`；脚本可一次性改发原始字节或拒绝。
fn handle(stream: &mut UnixStream, script: &Mutex<Script>) -> Option<Vec<u8>> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).ok()?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > FRAME_MAX_BYTES {
        return Some(refused(Verdict::Protocol));
    }
    let mut payload = vec![0u8; length];
    stream.read_exact(&mut payload).ok()?;
    let mut script = script.lock().expect("脚本未中毒");
    script.payloads.push(payload.clone());
    if ipc::payload_acceptable(&payload).is_err() {
        return Some(refused(Verdict::Protocol));
    }
    let Ok(request) = serde_json::from_slice::<Request>(&payload) else {
        return Some(refused(Verdict::Protocol));
    };
    script.requests.push(request.clone());
    if let Some(raw) = script.raw.take() {
        return Some(raw);
    }
    if let Some(verdict) = script.refuse.take() {
        return Some(refused(verdict));
    }
    Some(encode(&script.respond(&request)))
}

/// 把应答编成线上帧；用例也用它伪造「种类不符」的应答。
pub fn encode(response: &Response) -> Vec<u8> {
    let payload = serde_json::to_vec(response).expect("应答可序列化");
    ipc::encode_frame(&payload).expect("应答可成帧")
}

fn refused(verdict: Verdict) -> Vec<u8> {
    encode(&Response::Refused { verdict })
}

fn boot() -> Request {
    Request::BootId(Empty {})
}

#[test]
fn a_launch_frame_carries_only_closed_keys_and_never_the_argv_or_any_path() {
    let launcher = FakeLauncher::start();
    let approval = FakeApproval::new();
    let argv = build_argv(
        &approval.runtime,
        RunOperation::RunOnce,
        Path::new(MIRROR_ROOT),
    )
    .expect("可以构造");
    let request = Request::Launch(LaunchFrame {
        run_id: RunId::from_bytes([1u8; 16]),
        sequence: 1,
        operation: "run_once".to_owned(),
        mode: None,
        nonce: [2u8; 32],
        argv_digest: argv_digest(&argv),
        budget: budget(),
        payload_uid: PAYLOAD_UID,
        payload_gid: PAYLOAD_GID,
    });
    assert!(matches!(launcher.call(&request), Ok(Response::Launch(_))));

    let script = launcher.script();
    assert_eq!(script.requests, vec![request]);
    let payload = &script.payloads[0];
    let value: serde_json::Value = serde_json::from_slice(payload).expect("载荷是 JSON");
    for key in value.as_object().expect("载荷是对象").keys() {
        assert!(ALLOWED_KEYS.contains(&key.as_str()), "{key}");
    }
    let text = String::from_utf8(payload.clone()).expect("载荷是 UTF-8");
    for secret in [
        MIRROR_ROOT,
        "/opt/studiolo/node/bin/node",
        "cli.js",
        "\"argv\"",
        "--path",
    ] {
        assert!(!text.contains(secret), "{secret} 不得进帧");
    }
}

#[test]
fn every_refusal_verdict_maps_to_the_fixed_containment_gap() {
    let launcher = FakeLauncher::start();
    for verdict in [
        Verdict::Protocol,
        Verdict::Peer,
        Verdict::IdentityMismatch,
        Verdict::Busy,
        Verdict::ManagerUnavailable,
        Verdict::Budget,
        Verdict::NotProvisioned,
        Verdict::ArgvMismatch,
    ] {
        launcher.script().refuse = Some(verdict);
        assert_eq!(launcher.call(&boot()), Err(verdict.gap()), "{verdict:?}");
    }
    assert_eq!(
        launcher.call(&boot()),
        Ok(Response::BootId { boot_id: [7u8; 16] })
    );
}

#[test]
fn a_torn_oversized_or_foreign_reply_is_a_gap_never_a_fact() {
    let launcher = FakeLauncher::start();
    let oversized = u32::try_from(FRAME_MAX_BYTES + 1).expect("帧上限可表示");
    // 长度头越界或为零：不分配、不读取。
    launcher.script().raw = Some(oversized.to_be_bytes().to_vec());
    assert_eq!(launcher.call(&boot()), Err(ContainmentGap::FrameTooLarge));
    launcher.script().raw = Some(vec![0, 0, 0, 0]);
    assert_eq!(launcher.call(&boot()), Err(ContainmentGap::FrameTooLarge));
    // 声称 10 字节只给 3 字节就关连接。
    launcher.script().raw = Some(vec![0, 0, 0, 10, b'{', b'"', b'k']);
    assert_eq!(
        launcher.call(&boot()),
        Err(ContainmentGap::ManagerUnavailable)
    );
    // 顶层键不在闭集、形状不合法、不是对象：一律协议缺口。
    for bad in [
        br#"{"kind":"boot_id","boot_id":[7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7],"argv":["/bin/sh"]}"#
            .as_slice(),
        br#"{"kind":"boot_id"}"#.as_slice(),
        br#"{"kind":"nonsense"}"#.as_slice(),
        b"[1]".as_slice(),
    ] {
        launcher.script().raw = Some(ipc::encode_frame(bad).expect("可成帧"));
        assert_eq!(
            launcher.call(&boot()),
            Err(ContainmentGap::ProtocolViolation),
            "{bad:?}"
        );
    }
    assert_eq!(
        launcher.call(&boot()),
        Ok(Response::BootId { boot_id: [7u8; 16] })
    );
}

#[test]
fn a_missing_socket_is_not_provisioned_and_a_dead_or_foreign_endpoint_is_refused() {
    let directory = tempfile::tempdir().expect("临时目录可建");
    let socket = directory.path().join("launcher.sock");
    assert_eq!(
        client::call(&socket, &boot()),
        Err(ContainmentGap::TrustedLauncherNotProvisioned)
    );
    // 有 socket 文件但没人监听：manager 不可用，不是「未置备」。
    drop(UnixListener::bind(&socket).expect("可绑定"));
    assert_eq!(
        client::call(&socket, &boot()),
        Err(ContainmentGap::ManagerUnavailable)
    );
    // 端点基名不是固定名、或不是绝对规范路径：连都不连。
    assert_eq!(
        client::call(&directory.path().join("other.sock"), &boot()),
        Err(ContainmentGap::ProtocolViolation)
    );
    assert_eq!(
        client::call(Path::new("launcher.sock"), &boot()),
        Err(ContainmentGap::ProtocolViolation)
    );
}

#[test]
fn a_hung_launcher_times_out_instead_of_blocking_the_owner_forever() {
    let directory = tempfile::tempdir().expect("临时目录可建");
    let socket = directory.path().join("launcher.sock");
    let listener = UnixListener::bind(&socket).expect("可绑定");
    let (release, wait) = mpsc::channel::<()>();
    let hung = std::thread::spawn(move || {
        let accepted = listener.accept();
        let _ = wait.recv();
        drop(accepted);
    });
    let started = Instant::now();
    assert_eq!(
        client::call(&socket, &boot()),
        Err(ContainmentGap::ManagerUnavailable)
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_secs(4),
        "超时必须真的等过固定窗口：{elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(30),
        "超时不能无限拖长：{elapsed:?}"
    );
    release.send(()).expect("挂起线程仍在等");
    hung.join().expect("挂起线程可回收");
}
