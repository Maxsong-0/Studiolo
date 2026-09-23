//! 有界 JWKS 获取与缓存，生产取钥仅使用固定 HTTPS 源。

use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
    time::Duration,
};

use jsonwebtoken::DecodingKey;
use serde::Deserialize;
use tokio::{sync::Mutex, time::Instant};

use super::AccessError;

pub(super) const MAX_JWKS_BYTES: usize = 64 * 1024;
pub(super) const CACHE_TTL: Duration = Duration::from_secs(300);
pub(super) const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
pub(super) type Fetch = Pin<Box<dyn Future<Output = Result<Vec<u8>, AccessError>> + Send>>;

/// 内部 Seam：生产 HTTPS 与确定性内存夹具共享相同缓存/解析路径。
pub(super) trait KeySource: Send + Sync {
    fn fetch(&self) -> Fetch;
}

pub(super) struct HttpsSource {
    client: reqwest::Client,
    url: String,
}

impl HttpsSource {
    pub(super) fn new(issuer: &str) -> Result<Self, AccessError> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5))
            .pool_max_idle_per_host(1)
            .build()
            .map_err(|_| AccessError::Unavailable)?;
        Ok(Self {
            client,
            url: format!("{issuer}/cdn-cgi/access/certs"),
        })
    }
}

impl KeySource for HttpsSource {
    fn fetch(&self) -> Fetch {
        let client = self.client.clone();
        let url = self.url.clone();
        Box::pin(async move {
            let mut response = client
                .get(url)
                .send()
                .await
                .map_err(|_| AccessError::Unavailable)?;
            if response.status() != reqwest::StatusCode::OK
                || response
                    .content_length()
                    .is_some_and(|size| size > MAX_JWKS_BYTES as u64)
                || response
                    .headers()
                    .contains_key(reqwest::header::CONTENT_ENCODING)
            {
                return Err(AccessError::Unavailable);
            }
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| AccessError::Unavailable)?
            {
                if chunk.len() > MAX_JWKS_BYTES.saturating_sub(body.len()) {
                    return Err(AccessError::Unavailable);
                }
                body.extend_from_slice(&chunk);
            }
            Ok(body)
        })
    }
}

/// 某次取钥的结果与冷却记账；读写锁只包同步查表和赋值，绝不跨 await。
struct Snapshot {
    keys: BTreeMap<String, DecodingKey>,
    expires: Instant,
    last_attempt: Instant,
}

impl Snapshot {
    /// 仅在未过期时交出密钥；过期快照里的旧密钥一律不可用。
    fn fresh_key(&self, kid: &str, now: Instant) -> Option<DecodingKey> {
        if now < self.expires {
            self.keys.get(kid).cloned()
        } else {
            None
        }
    }
}

pub(super) struct KeyCache {
    source: Arc<dyn KeySource>,
    /// 当前快照。只有持有 `refresh` 闸门者才会写它，所以装入新密钥时不会与另一次刷新交错。
    snapshot: RwLock<Snapshot>,
    /// 刷新单飞闸门：冷却判定、冷却记账与取钥都在持闸期间进行；唯有它跨取钥的 await。
    refresh: Mutex<()>,
}

impl KeyCache {
    pub(super) async fn connect(source: Arc<dyn KeySource>) -> Result<Self, AccessError> {
        let keys = parse(&source.fetch().await?)?;
        let now = Instant::now();
        Ok(Self {
            source,
            snapshot: RwLock::new(Snapshot {
                keys,
                expires: now + CACHE_TTL,
                last_attempt: now,
            }),
            refresh: Mutex::new(()),
        })
    }

    pub(super) async fn key(&self, kid: &str) -> Result<DecodingKey, AccessError> {
        // 锁序：先刷新闸门、后快照读写锁，绝不在持快照锁时等闸门；快照锁只在同步代码里
        // 短暂持有、不跨任何 await，跨取钥 await 的只有闸门。
        // 快路径只拿读锁：未过期的已缓存 kid 即使另一请求正在取钥也立即返回，不等网络。
        let cached = self.read()?.fresh_key(kid, Instant::now());
        if let Some(key) = cached {
            return Ok(key);
        }
        // 慢路径持闸单飞：并发未知 kid 在闸门后排队，共用在途的那一次取钥。
        let _gate = self.refresh.lock().await;
        {
            // 排队期间别人可能已刷新好或已记下冷却，所以持闸后在写锁内按当下时刻重读快照；
            // 冷却判定与记账一次做完并在取钥前释放写锁，请求在取钥途中被取消，冷却记录也仍保留。
            let mut snapshot = self.write()?;
            let now = Instant::now();
            if let Some(key) = snapshot.fresh_key(kid, now) {
                return Ok(key);
            }
            if now.duration_since(snapshot.last_attempt) < REFRESH_INTERVAL {
                return Err(if now < snapshot.expires {
                    AccessError::Rejected
                } else {
                    AccessError::Unavailable
                });
            }
            snapshot.last_attempt = now;
        }
        // 取钥期间不持快照锁；失败时快照原样保留，过期密钥因此不会借失败复活。
        let keys = parse(&self.source.fetch().await?)?;
        let key = keys.get(kid).cloned();
        let expires = Instant::now() + CACHE_TTL;
        {
            let mut snapshot = self.write()?;
            snapshot.keys = keys;
            snapshot.expires = expires;
        }
        key.ok_or(AccessError::Rejected)
    }

    // 写锁内只做查表、时刻比较与字段赋值，均不会 panic，故不应中毒；万一中毒按失败关闭处理。
    fn read(&self) -> Result<RwLockReadGuard<'_, Snapshot>, AccessError> {
        self.snapshot.read().map_err(|_| AccessError::Unavailable)
    }

    fn write(&self) -> Result<RwLockWriteGuard<'_, Snapshot>, AccessError> {
        self.snapshot.write().map_err(|_| AccessError::Unavailable)
    }
}

#[derive(Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(Deserialize)]
struct Jwk {
    kid: String,
    kty: String,
    alg: String,
    #[serde(rename = "use")]
    usage: String,
    #[serde(default)]
    key_ops: Option<Vec<String>>,
    n: String,
    e: String,
}

pub(super) fn valid_kid(kid: &str) -> bool {
    !kid.is_empty()
        && kid.len() <= 128
        && kid
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

fn parse(bytes: &[u8]) -> Result<BTreeMap<String, DecodingKey>, AccessError> {
    if bytes.len() > MAX_JWKS_BYTES {
        return Err(AccessError::Unavailable);
    }
    let jwks: Jwks = serde_json::from_slice(bytes).map_err(|_| AccessError::Unavailable)?;
    if jwks.keys.is_empty() || jwks.keys.len() > 8 {
        return Err(AccessError::Unavailable);
    }
    let mut keys = BTreeMap::new();
    for key in jwks.keys {
        if !valid_kid(&key.kid)
            || key.kty != "RSA"
            || key.alg != "RS256"
            || key.usage != "sig"
            || key.key_ops.is_some_and(|ops| ops != ["verify"])
            || !(342..=1366).contains(&key.n.len())
            || key.e != "AQAB"
            || !key
                .n
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            return Err(AccessError::Unavailable);
        }
        let decoded = DecodingKey::from_rsa_components(&key.n, &key.e)
            .map_err(|_| AccessError::Unavailable)?;
        if keys.insert(key.kid, decoded).is_some() {
            return Err(AccessError::Unavailable);
        }
    }
    Ok(keys)
}

#[cfg(test)]
#[path = "keys_tests.rs"]
mod tests;
