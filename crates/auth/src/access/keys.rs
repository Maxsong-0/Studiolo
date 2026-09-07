//! 有界 JWKS 获取与缓存，生产取钥仅使用固定 HTTPS 源。

use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc, time::Duration};

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

struct Cached {
    keys: BTreeMap<String, DecodingKey>,
    expires: Instant,
    last_attempt: Instant,
}

pub(super) struct KeyCache {
    source: Arc<dyn KeySource>,
    cached: Mutex<Cached>,
}

impl KeyCache {
    pub(super) async fn connect(source: Arc<dyn KeySource>) -> Result<Self, AccessError> {
        let keys = parse(&source.fetch().await?)?;
        let now = Instant::now();
        Ok(Self {
            source,
            cached: Mutex::new(Cached {
                keys,
                expires: now + CACHE_TTL,
                last_attempt: now,
            }),
        })
    }

    pub(super) async fn key(&self, kid: &str) -> Result<DecodingKey, AccessError> {
        // 持锁跨刷新：并发未知 kid 共用一次取钥；取消请求后冷却记录仍保留。
        let mut cached = self.cached.lock().await;
        let now = Instant::now();
        let fresh = now < cached.expires;
        if fresh && let Some(key) = cached.keys.get(kid) {
            return Ok(key.clone());
        }
        if now.duration_since(cached.last_attempt) < REFRESH_INTERVAL {
            return Err(if fresh {
                AccessError::Rejected
            } else {
                AccessError::Unavailable
            });
        }
        cached.last_attempt = now;
        let keys = parse(&self.source.fetch().await?)?;
        cached.keys = keys;
        cached.expires = Instant::now() + CACHE_TTL;
        cached.keys.get(kid).cloned().ok_or(AccessError::Rejected)
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
