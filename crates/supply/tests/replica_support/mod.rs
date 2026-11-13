//! 私有不可变副本夹具，供供给层及真实 MCP 契约复用。
#![allow(dead_code)]

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use tempfile::TempDir;

pub const FIRST: &str = "00000000000000000000000000000001";
pub const SECOND: &str = "00000000000000000000000000000002";

pub struct ReplicaFixture {
    pub temp: TempDir,
    pub root: PathBuf,
    pub observed: i64,
}

pub fn private_dir(path: &Path) {
    if !path.exists() {
        private_dir(path.parent().unwrap());
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
}

pub fn write(path: &Path, body: &[u8]) {
    private_dir(path.parent().unwrap());
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

pub fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl ReplicaFixture {
    pub fn new() -> Self {
        let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let root = temp.path().join("replica");
        private_dir(&root);
        let fixture = Self {
            temp,
            root,
            observed: jiff::Timestamp::now().as_millisecond() - 10_000,
        };
        fixture.publish(FIRST, "oldmarker", fixture.observed);
        fixture
    }

    pub fn release(&self, generation: &str) -> PathBuf {
        self.root.join("releases").join(generation)
    }

    pub fn file(&self, relative: &str) -> PathBuf {
        self.release(FIRST).join("vault").join(relative)
    }

    pub fn state(&self) -> Value {
        serde_json::from_slice(&fs::read(self.root.join("control/state.json")).unwrap()).unwrap()
    }

    pub fn write_state(&self, state: &Value) {
        write(
            &self.root.join("control/state.json"),
            &serde_json::to_vec(state).unwrap(),
        );
    }

    pub fn publish(&self, generation: &str, marker: &str, observed: i64) {
        let note = format!("# {marker}\n\n{marker} content\n");
        // 标记同时写进「状态」：路牌清单化后只摘状态与未关闭 BUG，
        // 仅放「当前版本」的话标记不会出现在路牌里，跟随性断言就失去观察点。
        let index = format!(
            "# Project\n\n| 项 | 当前值 |\n|---|---|\n| 当前版本 | {marker} |\n| 状态 | {marker} |\n"
        );
        self.publish_entries(
            generation,
            observed,
            &[
                ("project/00-总览", None),
                ("project/00-总览/INDEX.md", Some(index.as_bytes())),
                ("project/empty", None),
                ("project/image.bin", Some(&[0, 1, 255][..])),
                ("project/note.md", Some(note.as_bytes())),
            ],
        );
    }

    /// 按给定条目发布一个代次并让状态指向它；路径须按字节序排好，`None` 表示目录。
    pub fn publish_entries(
        &self,
        generation: &str,
        observed: i64,
        entries: &[(&str, Option<&[u8]>)],
    ) {
        let release = self.release(generation);
        let entries = entries.iter().map(|&(path, content)| {
            let target = release.join("vault").join(path);
            match content {
                Some(body) => { write(&target, body); json!({"path": path, "kind": "file", "bytes": body.len(), "sha256": digest(body)}) }
                None => { private_dir(&target); json!({"path": path, "kind": "dir", "bytes": 0, "sha256": null}) }
            }
        }).collect::<Vec<_>>();
        let manifest = serde_json::to_vec(
            &json!({"schema_version": 1, "allow": ["project"], "entries": entries}),
        )
        .unwrap();
        write(&release.join("manifest.json"), &manifest);
        self.write_state(&json!({"schema_version": 1, "status": "ready", "last_attempt_at_ms": observed + 100, "last_error": null,
            "receipt": {"generation": generation, "source_digest": digest(&manifest), "source_observed_at_ms": observed, "received_at_ms": observed + 100}}));
    }

    pub fn rewrite_manifest(&self, mutate: impl FnOnce(&mut Value)) {
        let path = self.release(FIRST).join("manifest.json");
        let mut value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        mutate(&mut value);
        let raw = serde_json::to_vec(&value).unwrap();
        write(&path, &raw);
        let mut state = self.state();
        state["receipt"]["source_digest"] = json!(digest(&raw));
        self.write_state(&state);
    }
}
