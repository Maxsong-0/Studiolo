//! 验证后的派生模型缓存：无源读取能力，缓存不决定正文、白名单、身份或新鲜度。
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use studiolo_supply::{
    ParseCache, Snapshot, SupplyError, WorkbenchReadModel,
    index_cache::{IndexCacheStore, validate_isolation},
};

use crate::{options::ServeOptions, state::Generation};

/// 当前模型的来源，仅用于本机诊断与验收。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelSource {
    /// 从本次输入解析、派生。
    Rebuilt,
    /// 使用持久缓存中的单篇解析结果，重新构建全局关联。
    Disk,
    /// 输入未变，直接共享已验证模型。
    Memory,
    /// 输入变化，复用进程内未变文档的单篇解析结果。
    Incremental,
}

/// 最近一次持久化操作结果，不表示每次请求都重新检查了磁盘缓存。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Persistence {
    /// 未配置持久缓存。
    Disabled,
    /// 最近一次读取或写入成功。
    Ready,
    /// 缓存格式、读写或编码预算异常，读取模型仍来自有效源。
    Unavailable,
}

/// 派生索引运行结果；不进入2000token路牌或内容响应。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexStatus {
    /// 模型来源。
    pub source: ModelSource,
    /// 本次真正执行单篇解析的文档数。
    pub parsed: usize,
    /// 本次复用的文档数。
    pub reused: usize,
    /// 最近持久操作状态。
    pub persistence: Persistence,
}

struct State {
    cache: Option<ParseCache>,
    loaded: bool,
    persistence: Persistence,
}

pub(crate) struct IndexRuntime {
    store: Option<IndexCacheStore>,
    scope: String,
    state: Mutex<State>,
}

impl IndexRuntime {
    pub(crate) fn open(options: &ServeOptions) -> Result<Self, String> {
        let (store, persistence, scope) = if let Some(root) = &options.index_cache {
            let source = options
                .vault
                .as_ref()
                .or(options.replica_root.as_ref())
                .ok_or("供给源未指定")?;
            validate_isolation(root, &[source.as_path(), options.data.as_path()])
                .map_err(|_| "索引缓存目录不安全或与源/身份目录重叠".to_owned())?;
            let mut allow = options.allow.clone();
            allow.sort();
            let scope_bytes = serde_json::to_vec(&(
                "studiolo-index-scope-v1",
                options.replica_root.is_some(),
                source.to_str().ok_or("索引来源路径不是UTF-8")?,
                allow,
            ))
            .map_err(|_| "索引范围编码失败".to_owned())?;
            let scope = Sha256::digest(scope_bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            match IndexCacheStore::open(root) {
                Ok(store) => (Some(store), Persistence::Ready, scope),
                Err(SupplyError::CorruptStore | SupplyError::StoreUnavailable) => {
                    (None, Persistence::Unavailable, scope)
                }
                Err(_) => return Err("索引缓存私有存储边界拒绝启动".to_owned()),
            }
        } else {
            (None, Persistence::Disabled, String::new())
        };
        Ok(Self {
            store,
            scope,
            state: Mutex::new(State {
                cache: None,
                loaded: false,
                persistence,
            }),
        })
    }

    pub(crate) fn build(
        &self,
        snapshot: &Snapshot,
        previous: Option<&Generation>,
    ) -> Result<(Arc<WorkbenchReadModel>, IndexStatus), String> {
        // 调用方已完成当前源的完整验证；不以缓存、时间戳或回执短路源验证。
        if let Some(previous) = previous
            && previous.snapshot.notes == snapshot.notes
        {
            return Ok((
                previous.model.clone(),
                IndexStatus {
                    source: ModelSource::Memory,
                    parsed: 0,
                    reused: snapshot.notes.len(),
                    persistence: previous.index_status.persistence,
                },
            ));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "索引缓存锁不可用".to_owned())?;
        let mut disk = false;
        if !state.loaded {
            state.loaded = true;
            if let Some(store) = &self.store {
                match store.load(&self.scope) {
                    Ok(Some(bytes)) => match ParseCache::decode(&bytes) {
                        Ok(cache) => {
                            state.cache = Some(cache);
                            disk = true;
                        }
                        Err(_) => state.persistence = Persistence::Unavailable,
                    },
                    Ok(None) => {}
                    Err(SupplyError::UnsafeStorage) if previous.is_none() => {
                        return Err("索引缓存私有存储边界拒绝启动".to_owned());
                    }
                    Err(_) => state.persistence = Persistence::Unavailable,
                }
            }
        }
        let documents = snapshot.documents();
        let (model, cache, stats) =
            WorkbenchReadModel::build_cached(&documents, state.cache.as_ref())
                .map_err(|_| "读模型构建失败".to_owned())?;
        if let Some(store) = &self.store {
            state.persistence = match cache.encode() {
                Ok(bytes) => match store.replace(&self.scope, &bytes) {
                    Ok(()) => Persistence::Ready,
                    Err(SupplyError::UnsafeStorage) if previous.is_none() => {
                        return Err("索引缓存私有存储边界拒绝启动".to_owned());
                    }
                    Err(_) => Persistence::Unavailable,
                },
                Err(_) => Persistence::Unavailable,
            };
        }
        state.cache = Some(cache);
        let source = if stats.reused == 0 {
            ModelSource::Rebuilt
        } else if disk {
            ModelSource::Disk
        } else {
            ModelSource::Incremental
        };
        Ok((
            Arc::new(model),
            IndexStatus {
                source,
                parsed: stats.parsed,
                reused: stats.reused,
                persistence: state.persistence,
            },
        ))
    }
}
