//! 持久索引集成测试只使用相互隔离的临时源、身份库与缓存目录。

use crate::replica_support::{private_dir, write};
use std::path::{Path, PathBuf};
use studiolo_agent_server::options::ServeOptions;
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_supply::AgentStore;
use tempfile::TempDir;

pub struct Fixture {
    pub temp: TempDir,
    pub vault: PathBuf,
    pub cache: PathBuf,
    pub options: ServeOptions,
}

pub fn initialize(data: &Path) {
    private_dir(data);
    private_dir(&data.join("audit"));
    drop(AgentStore::open(data).unwrap());
    drop(SqliteAuditLedger::open(&data.join("audit")).unwrap());
}

impl Fixture {
    pub fn new() -> Self {
        let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let vault = temp.path().join("vault");
        let cache = temp.path().join("cache");
        private_dir(&vault.join("project"));
        private_dir(&cache);
        let data = temp.path().join("data");
        initialize(&data);
        let options = ServeOptions {
            vault: Some(vault.clone()),
            replica_root: None,
            replica_max_age_secs: 300,
            allow: vec!["project".into()],
            data,
            index_cache: Some(cache.clone()),
            port: 0,
        };
        Self {
            temp,
            vault,
            cache,
            options,
        }
    }

    pub fn note(&self, name: &str, body: &str) {
        write(&self.vault.join("project").join(name), body.as_bytes());
    }
}

pub fn replica_options(replica: &crate::replica_support::ReplicaFixture) -> ServeOptions {
    let cache = replica.temp.path().join("index-cache");
    private_dir(&cache);
    let data = replica.temp.path().join("agent-data");
    initialize(&data);
    ServeOptions {
        vault: None,
        replica_root: Some(replica.root.clone()),
        replica_max_age_secs: 300,
        allow: vec!["project".into()],
        data,
        index_cache: Some(cache),
        port: 0,
    }
}
