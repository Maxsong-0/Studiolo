//! 全部根均为临时私有目录；文件源夹具按帧读取，正文身份由生产 freeze_source 核验。

use std::{
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
};
use studiolo_audit_ledger::MemoryAuditLedger;
use studiolo_core::{Revision, VaultPath};
use studiolo_host_config::Config;
use studiolo_local_server::CanonicalWorkbench;
use studiolo_local_server::sync_plane::driver::content::inbound::freeze_source;
use studiolo_local_server::sync_plane::mirror::MirrorView;
use studiolo_local_server::sync_plane::mirror::stage::StageStore;
use studiolo_sync_bridge::executor::{MirrorContent, TransferFault};
use studiolo_sync_bridge::{FileVersion, Side};
use studiolo_vault_broker::FsVaultBroker;

pub const NOW: i64 = 1_790_000_000;
pub const SEQUENCE: u64 = 7;

pub fn path() -> VaultPath {
    VaultPath::parse("notes/body.txt").unwrap()
}

pub fn version(bytes: &[u8]) -> FileVersion {
    use sha2::{Digest, Sha256};
    let sha256: [u8; 32] = Sha256::digest(bytes).into();
    FileVersion {
        revision: sha256,
        sha256,
        bytes: bytes.len() as u64,
    }
}

pub fn private(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

pub fn view(path: &Path) -> MirrorView {
    let metadata = fs::metadata(path).unwrap();
    MirrorView::new(path.to_owned(), (metadata.dev(), metadata.ino()))
}

pub struct Source {
    root: PathBuf,
    open: Option<fs::File>,
}

impl Source {
    pub fn new(root: PathBuf) -> Self {
        Self { root, open: None }
    }
}

impl MirrorContent for Source {
    fn open(&mut self, side: Side, path: &VaultPath, _: &FileVersion) -> Result<(), TransferFault> {
        assert_eq!(side, Side::Mirror);
        self.open = Some(
            fs::File::open(self.root.join(path.as_str()))
                .map_err(|_| TransferFault::Unavailable)?,
        );
        Ok(())
    }

    fn next_frame(&mut self, buffer: &mut [u8]) -> Result<usize, TransferFault> {
        self.open
            .as_mut()
            .ok_or(TransferFault::Unavailable)?
            .read(buffer)
            .map_err(|_| TransferFault::Unavailable)
    }
}

pub struct Fixture {
    pub root: PathBuf,
    pub previous: Revision,
    pub audit: Arc<MemoryAuditLedger>,
    _temp: tempfile::TempDir,
}

impl Fixture {
    pub fn new(bytes: &[u8]) -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize("/tmp").unwrap()).unwrap();
        let root = temp.path().to_owned();
        for name in [
            "vault", "data", "assets", "config", "auth", "audit", "plane", "mirror",
        ] {
            private(&root.join(name));
        }
        for name in ["vault", "mirror"] {
            private(&root.join(name).join("notes"));
        }
        fs::write(
            root.join("vault/notes/body.txt"),
            b"canonical before bridge",
        )
        .unwrap();
        fs::write(root.join("mirror/notes/body.txt"), bytes).unwrap();
        let broker = FsVaultBroker::open(&root.join("vault"), &root.join("data")).unwrap();
        let previous = broker.read_file(&path(), 1024).unwrap().revision;
        drop(broker);
        let config = root.join("config/owner.json");
        fs::write(&config, serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "auth_directory": root.join("auth"),
            "audit_directory": root.join("audit"),
            "owner": {"issuer": "https://fixture.cloudflareaccess.com", "audience": "fixture-audience",
                "subject": "fictional-person", "email": "owner@example.invalid"},
            "idle_seconds": 1800, "absolute_seconds": 43200, "seed_ref": "synthetic-seed"
        })).unwrap()).unwrap();
        fs::set_permissions(config, fs::Permissions::from_mode(0o600)).unwrap();
        StageStore::initialize(&root.join("plane")).unwrap();
        Self {
            root,
            previous,
            audit: Arc::new(MemoryAuditLedger::new()),
            _temp: temp,
        }
    }

    pub async fn writer(&self) -> Arc<CanonicalWorkbench> {
        let roots = Config::load(&self.root.join("config/owner.json"))
            .unwrap()
            .canonical_serving_roots(
                &self.root.join("vault"),
                &self.root.join("assets"),
                &self.root.join("data"),
            )
            .unwrap();
        let broker =
            FsVaultBroker::open_existing(&self.root.join("vault"), &self.root.join("data"))
                .unwrap();
        CanonicalWorkbench::prepare_with_audit(broker, Arc::new(roots), self.audit.clone())
            .await
            .unwrap()
    }

    pub fn stage(&self) -> StageStore {
        StageStore::open(&self.root.join("plane")).unwrap()
    }

    pub fn freeze(&self, stage: &mut StageStore, version: &FileVersion) {
        let mut source = Source::new(self.root.join("mirror"));
        freeze_source(stage, &mut source, &path(), version, SEQUENCE, NOW).unwrap();
    }

    pub fn commits(&self) -> u64 {
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(self.root.join("data/writer-state.json")).unwrap())
                .unwrap();
        value["committed_through"].as_u64().unwrap()
    }
}
