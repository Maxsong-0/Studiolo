//! 外 stage（ADR-025 D8）：`<plane_root>/stage/objects/<sha256 hex>` 按内容寻址落库，
//! 删除前保全的对象与 7 天保留期记在固定名 `stage/index.json`。
//!
//! **本文件是 `sync_plane` 下唯一允许做真实文件 IO 的地方**（架构守卫按固定文件名登记）。
//! 写法照 `observability` 的 `deadman-state.json`：临时件 → fsync → renameat → fsync 目录；
//! 读时失败关闭。对象文件 NOFOLLOW、O_EXCL、0600；孤儿在打开时回收；
//! 保留期到且不被当前 pending 引用的对象才删。

use super::super::PlaneError;
use std::path::{Path, PathBuf};
use studiolo_sync_bridge::executor::ObjectId;

/// 平面私有根下的 stage 目录名。
pub const STAGE_DIR: &str = "stage";
/// stage 内的对象目录名。
pub const OBJECTS_DIR: &str = "objects";
/// 保全索引的固定文件名。
pub const INDEX_FILE: &str = "index.json";
/// 保全索引的唯一暂存件名；只用于完整替换。
pub const INDEX_STAGE_FILE: &str = "index.json.new";
/// 接收中对象的唯一临时名；同一时刻最多一个接收。
pub const RECEIVING_FILE: &str = "receiving.partial";
/// 删除前保全的最短保留期。
pub const RETENTION_SECONDS: u64 = 7 * 24 * 60 * 60;
/// 索引文件字节上限。
pub const INDEX_MAX_BYTES: usize = 1024 * 1024;

/// 保全索引里的一条：谁保全的、保全时刻、是否仍被引用。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreservedEntry {
    /// 对象地址（sha256 小写十六进制）。
    pub object: String,
    /// 对象字节数。
    pub bytes: u64,
    /// 保全时刻（Unix 秒）。
    pub preserved_at_unix: i64,
    /// 产生保全的计划序号；退休后可回收。
    pub plan_sequence: u64,
    /// 原逻辑路径（只用于人工找回，不参与任何判定）。
    pub original_path: String,
}

/// 固定名索引文档：版本 1，带校验和，失败关闭。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageIndex {
    /// 固定为 1。
    pub version: u8,
    /// 保全条目；按对象地址排序。
    pub preserved: Vec<PreservedEntry>,
    /// 对上面字段的域分隔 sha256（小写十六进制）。
    pub checksum: String,
}

/// 已打开的外 stage；持有根目录 FD 与解析后的索引。
pub struct StageStore {
    root: PathBuf,
    index: StageIndex,
    receiving: bool,
}

impl StageStore {
    /// 打开 `<plane_root>/stage`：核对 0700／本人所有，读索引（失败关闭），回收孤儿。
    ///
    /// # Errors
    /// 目录或索引不满足约束返回 `Storage`。
    pub fn open(plane_root: &Path) -> Result<Self, PlaneError> {
        // B 轮 W4 实现（真实 IO 只许出现在本文件）。
        let _ = plane_root;
        Err(PlaneError::Storage)
    }

    /// 管理员显式初始化：目录与空索引都不存在时才建。
    ///
    /// # Errors
    /// 已存在返回 `Storage`。
    pub fn initialize(plane_root: &Path) -> Result<(), PlaneError> {
        // B 轮 W4 实现；`studiolo-maintain sync-init`（W6）调用它。
        let _ = plane_root;
        Err(PlaneError::Storage)
    }

    /// 开始接收一个对象：独占创建 `receiving.partial`。
    ///
    /// # Errors
    /// 已有接收中对象返回 `Storage`。
    pub fn begin(&mut self) -> Result<(), PlaneError> {
        // B 轮 W4 实现。
        if self.receiving {
            return Err(PlaneError::Storage);
        }
        Err(PlaneError::Storage)
    }

    /// 追加一帧到接收中对象。
    ///
    /// # Errors
    /// 没有接收中对象或写失败返回 `Storage`。
    pub fn accept(&mut self, frame: &[u8]) -> Result<(), PlaneError> {
        // B 轮 W4 实现。
        let _ = frame;
        Err(PlaneError::Storage)
    }

    /// 按最终地址封口：fsync → renameat 到 `objects/<hex>` → fsync 目录。
    ///
    /// # Errors
    /// 地址已存在（重复落库不占新条目）时静默成功；IO 失败返回 `Storage`。
    pub fn finish(&mut self, id: ObjectId) -> Result<(), PlaneError> {
        // B 轮 W4 实现。
        let _ = id;
        Err(PlaneError::Storage)
    }

    /// 放弃接收中对象并删除临时件。
    ///
    /// # Errors
    /// IO 失败返回 `Storage`。
    pub fn discard(&mut self) -> Result<(), PlaneError> {
        // B 轮 W4 实现。
        Err(PlaneError::Storage)
    }

    /// 把已封口对象登记为「删除前保全」，进入 7 天保留期。
    ///
    /// # Errors
    /// 对象不存在或索引写失败返回 `Storage`。
    pub fn preserve(
        &mut self,
        id: ObjectId,
        plan_sequence: u64,
        original_path: &str,
        now_unix: i64,
    ) -> Result<(), PlaneError> {
        // B 轮 W4 实现。
        let _ = (id, plan_sequence, original_path, now_unix);
        Err(PlaneError::Storage)
    }

    /// 对象文件路径；只读用途（安装时 renameat 的源）。
    #[must_use]
    pub fn object_path(&self, id: ObjectId) -> PathBuf {
        self.root.join(OBJECTS_DIR).join(hex(id.digest()))
    }

    /// 回收：保留期已到且 `plan_sequence` ≤ `retired_through` 的保全对象，以及索引外的孤儿
    /// （接收中除外）。返回回收的对象数。
    ///
    /// # Errors
    /// IO 失败返回 `Storage`。
    pub fn sweep(&mut self, now_unix: i64, retired_through: u64) -> Result<usize, PlaneError> {
        // B 轮 W4 实现。
        let _ = (
            now_unix,
            retired_through,
            RETENTION_SECONDS,
            INDEX_MAX_BYTES,
        );
        Err(PlaneError::Storage)
    }

    /// 当前索引（只读）。
    #[must_use]
    pub fn index(&self) -> &StageIndex {
        &self.index
    }
}

/// 小写十六进制。
#[must_use]
pub fn hex(bytes: [u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(64);
    for byte in bytes {
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}
