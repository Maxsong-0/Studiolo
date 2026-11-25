//! 收旧槽的真实链路（T-239）：真 owner、真传输存储、真审计账本，假 launcher 只答 `probe`。
//! 证明停在 `Prepared` 的运行收成「没执行」的全程没有向 launcher 发过任何动作请求。

use std::io::{Read, Write};
use std::num::NonZeroU64;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::{fs, time::Duration};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditPhase, SqliteAuditLedger};
use studiolo_core::VaultUuid;
use studiolo_local_server::sync_plane::driver::headless::{OwnerSlot, settle_active};
use studiolo_local_server::sync_transport_owner::flush_transport_audit;
use studiolo_sync_transport::linux::protocol::{
    PROTOCOL_VERSION, ProbeReport, Request, Response, Verdict,
};
use studiolo_sync_transport::linux::{ipc, verify_backend_at};
use studiolo_sync_transport::owned_run::{
    ApprovedTransport, BackendDescriptor, ExitObservation, PINNED_RUNTIME_VERSION, PinnedRuntime,
    Quiescence, Result as TransportResult, RootPins, RunBudget, RunOperation, RunPhase,
    RuntimePins, SyncTransportOwner, TRANSPORT_RUN_ACTION, TransportBindingFacts, initialize_store,
};

const PAYLOAD_ID: u32 = 61001;

/// 固定批准：真实目录、真实 `(dev, ino)`，其余是批准值。
struct Approval {
    vault_uuid: VaultUuid,
    private_root: PathBuf,
    mirror_root: PathBuf,
}

fn identity(path: &Path) -> (u64, u64) {
    let metadata = fs::metadata(path).unwrap();
    (metadata.dev(), metadata.ino())
}

impl ApprovedTransport for Approval {
    fn revalidate(&self) -> TransportResult<()> {
        Ok(())
    }

    fn binding(&self) -> TransportResult<TransportBindingFacts> {
        Ok(TransportBindingFacts {
            vault_uuid: self.vault_uuid,
            link_id: [1; 16],
            remote_vault: [2; 16],
            scope_epoch: NonZeroU64::MIN,
            scope_hash: [3; 32],
            approved_config_digest: [4; 32],
        })
    }

    fn roots(&self) -> RootPins {
        RootPins {
            private_root: self.private_root.clone(),
            private_identity: identity(&self.private_root),
            mirror_root: self.mirror_root.clone(),
            mirror_identity: identity(&self.mirror_root),
            mirror_view_root: self.mirror_root.with_file_name("mirror-view"),
        }
    }

    fn runtime(&self) -> RuntimePins {
        RuntimePins {
            version: PINNED_RUNTIME_VERSION.to_owned(),
            node_path: PathBuf::from("/opt/studiolo/node/bin/node"),
            cli_path: PathBuf::from("/opt/studiolo/obsidian-headless/cli.js"),
            closure_digest: [5; 32],
        }
    }

    fn backend(&self) -> BackendDescriptor {
        BackendDescriptor {
            service_unit: "studiolo-sync-launcher@link-1.service".to_owned(),
            payload_uid: PAYLOAD_ID,
            payload_gid: PAYLOAD_ID,
            payload_cgroup_prefix: "/sys/fs/cgroup/studiolo.slice/payload".to_owned(),
            digest: [6; 32],
        }
    }

    fn budget(&self) -> RunBudget {
        RunBudget {
            deadline_seconds: 3600,
            stop_grace_seconds: 30,
            stdout_bytes: 262_144,
            stderr_bytes: 262_144,
            config_json_bytes: 65_536,
            private_state_bytes: 67_108_864,
            log_bytes: 268_435_456,
            memory_bytes: 2_147_483_648,
            pids_max: 64,
            cpu_max_percent: 200,
        }
    }
}

/// 只答 `probe` 的假 launcher；别的请求一律拒绝，收到过什么全记下。
struct ProbeOnlyLauncher {
    _directory: tempfile::TempDir,
    socket: PathBuf,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ProbeOnlyLauncher {
    fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("launcher.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (requests, stop) = (Arc::clone(&requests), Arc::clone(&stop));
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(mut stream) = stream else { break };
                    answer(&mut stream, &requests);
                }
            })
        };
        Self {
            _directory: directory,
            socket,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for ProbeOnlyLauncher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // 自己连一次，叫醒阻塞在 accept 的线程。
        let _ = UnixStream::connect(&self.socket);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn answer(stream: &mut UnixStream, requests: &Mutex<Vec<Request>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut header = [0; 4];
    if stream.read_exact(&mut header).is_err() {
        return;
    }
    let mut payload = vec![0; u32::from_be_bytes(header) as usize];
    if stream.read_exact(&mut payload).is_err() {
        return;
    }
    let request: Request = serde_json::from_slice(&payload).unwrap();
    requests.lock().unwrap().push(request.clone());
    let response = match request {
        Request::Probe { nonce } => Response::Probe(ProbeReport {
            nonce,
            protocol_version: PROTOCOL_VERSION,
            systemd_version: 258,
            cgroup_v2: true,
            loop_quota: true,
            dedicated_identity: true,
            payload_uid: PAYLOAD_ID,
            payload_gid: PAYLOAD_ID,
            boot_id: [7; 16],
        }),
        _ => Response::Refused {
            verdict: Verdict::Protocol,
        },
    };
    let body = serde_json::to_vec(&response).unwrap();
    let _ = stream.write_all(&ipc::encode_frame(&body).unwrap());
}

struct Fixture {
    _directory: tempfile::TempDir,
    approval: Approval,
    launcher: ProbeOnlyLauncher,
    ledger: SqliteAuditLedger,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(directory.path()).unwrap();
        for name in ["transport", "mirror", "audit"] {
            fs::create_dir(base.join(name)).unwrap();
            fs::set_permissions(base.join(name), fs::Permissions::from_mode(0o700)).unwrap();
        }
        let approval = Approval {
            vault_uuid: VaultUuid::new(),
            private_root: base.join("transport"),
            mirror_root: base.join("mirror"),
        };
        let runtime = PinnedRuntime::from_approval(&approval).unwrap();
        initialize_store(&approval, &runtime).unwrap();
        Self {
            ledger: SqliteAuditLedger::open(&base.join("audit")).unwrap(),
            _directory: directory,
            approval,
            launcher: ProbeOnlyLauncher::start(),
        }
    }

    fn open(&self) -> SyncTransportOwner<'_> {
        let backend = verify_backend_at(&self.approval, &self.launcher.socket).unwrap();
        SyncTransportOwner::open_existing(
            &self.approval,
            PinnedRuntime::from_approval(&self.approval).unwrap(),
            backend,
        )
        .unwrap()
    }

    fn phases(&self) -> Vec<AuditPhase> {
        self.ledger
            .query(&AuditFilter::default().with_action(TRANSPORT_RUN_ACTION))
            .unwrap()
            .iter()
            .map(|record| record.event().phase())
            .collect()
    }
}

#[test]
fn a_run_left_prepared_on_disk_is_closed_through_the_real_owner_and_ledger() {
    for intent_acknowledged in [false, true] {
        let fixture = Fixture::new();
        // 上一个进程登记了运行、还没发启动就没了；意图回执写没写成都可能。
        {
            let mut owner = fixture.open();
            owner.prepare_once(1, RunOperation::RunOnce, 1).unwrap();
            if intent_acknowledged {
                flush_transport_audit(&mut owner, &fixture.ledger, 2).unwrap();
            }
        }
        let mut owner = fixture.open();
        let run_id = owner.inspect().unwrap().active.expect("活动槽仍在").run_id;

        settle_active(&mut OwnerSlot::new(&mut owner, &fixture.ledger)).unwrap();

        let status = owner.inspect().unwrap();
        assert!(status.active.is_none());
        let terminal = status.last_terminal.expect("已审计闭合摘要");
        assert_eq!(terminal.run_id, run_id);
        assert_eq!(terminal.observation.phase, RunPhase::Closed);
        assert_eq!(terminal.observation.exit, ExitObservation::NotObserved);
        assert_eq!(terminal.observation.quiescence, Quiescence::Unconfirmed);
        assert!(terminal.container.is_none());
        assert_eq!(
            fixture.phases(),
            vec![AuditPhase::Intent, AuditPhase::Outcome]
        );
        // 再收一遍什么都不发生；全程只探测过，没有任何动作请求。
        settle_active(&mut OwnerSlot::new(&mut owner, &fixture.ledger)).unwrap();
        assert_eq!(fixture.phases().len(), 2);
        let requests = fixture.launcher.requests();
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|request| matches!(request, Request::Probe { .. }))
        );
    }
}
