//! 仅 cfg(test) 编译的签名夹具；公开测试密钥来自 jsonwebtoken 10.3.0 MIT 测试。

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;

use super::{
    AccessConfig, AccessError, AccessVerifier,
    keys::{Fetch, KeyCache, KeySource},
};

pub(super) const PRIVATE_KEY: &[u8] = b"-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDJETqse41HRBsc
7cfcq3ak4oZWFCoZlcic525A3FfO4qW9BMtRO/iXiyCCHn8JhiL9y8j5JdVP2Q9Z
IpfElcFd3/guS9w+5RqQGgCR+H56IVUyHZWtTJbKPcwWXQdNUX0rBFcsBzCRESJL
eelOEdHIjG7LRkx5l/FUvlqsyHDVJEQsHwegZ8b8C0fz0EgT2MMEdn10t6Ur1rXz
jMB/wvCg8vG8lvciXmedyo9xJ8oMOh0wUEgxziVDMMovmC+aJctcHUAYubwoGN8T
yzcvnGqL7JSh36Pwy28iPzXZ2RLhAyJFU39vLaHdljwthUaupldlNyCfa6Ofy4qN
ctlUPlN1AgMBAAECggEAdESTQjQ70O8QIp1ZSkCYXeZjuhj081CK7jhhp/4ChK7J
GlFQZMwiBze7d6K84TwAtfQGZhQ7km25E1kOm+3hIDCoKdVSKch/oL54f/BK6sKl
qlIzQEAenho4DuKCm3I4yAw9gEc0DV70DuMTR0LEpYyXcNJY3KNBOTjN5EYQAR9s
2MeurpgK2MdJlIuZaIbzSGd+diiz2E6vkmcufJLtmYUT/k/ddWvEtz+1DnO6bRHh
xuuDMeJA/lGB/EYloSLtdyCF6sII6C6slJJtgfb0bPy7l8VtL5iDyz46IKyzdyzW
tKAn394dm7MYR1RlUBEfqFUyNK7C+pVMVoTwCC2V4QKBgQD64syfiQ2oeUlLYDm4
CcKSP3RnES02bcTyEDFSuGyyS1jldI4A8GXHJ/lG5EYgiYa1RUivge4lJrlNfjyf
dV230xgKms7+JiXqag1FI+3mqjAgg4mYiNjaao8N8O3/PD59wMPeWYImsWXNyeHS
55rUKiHERtCcvdzKl4u35ZtTqQKBgQDNKnX2bVqOJ4WSqCgHRhOm386ugPHfy+8j
m6cicmUR46ND6ggBB03bCnEG9OtGisxTo/TuYVRu3WP4KjoJs2LD5fwdwJqpgtHl
yVsk45Y1Hfo+7M6lAuR8rzCi6kHHNb0HyBmZjysHWZsn79ZM+sQnLpgaYgQGRbKV
DZWlbw7g7QKBgQCl1u+98UGXAP1jFutwbPsx40IVszP4y5ypCe0gqgon3UiY/G+1
zTLp79GGe/SjI2VpQ7AlW7TI2A0bXXvDSDi3/5Dfya9ULnFXv9yfvH1QwWToySpW
Kvd1gYSoiX84/WCtjZOr0e0HmLIb0vw0hqZA4szJSqoxQgvF22EfIWaIaQKBgQCf
34+OmMYw8fEvSCPxDxVvOwW2i7pvV14hFEDYIeZKW2W1HWBhVMzBfFB5SE8yaCQy
pRfOzj9aKOCm2FjjiErVNpkQoi6jGtLvScnhZAt/lr2TXTrl8OwVkPrIaN0bG/AS
aUYxmBPCpXu3UjhfQiWqFq/mFyzlqlgvuCc9g95HPQKBgAscKP8mLxdKwOgX8yFW
GcZ0izY/30012ajdHY+/QK5lsMoxTnn0skdS+spLxaS5ZEO4qvPVb8RAoCkWMMal
2pOhmquJQVDPDLuZHdrIiKiDM20dy9sMfHygWcZjQ4WSxf/J7T9canLZIXFhHAZT
3wc9h4G8BBCtWN2TN/LsGZdB
-----END PRIVATE KEY-----";

pub(super) fn config() -> AccessConfig {
    AccessConfig::new(
        "https://studiolo.example.com".into(),
        "https://studiolo.cloudflareaccess.com".into(),
        "example-audience".into(),
        "maxsong@example.com".into(),
    )
    .unwrap()
}

pub(super) fn claims() -> Value {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    json!({"iss":"https://studiolo.cloudflareaccess.com", "aud":["example-audience"],
        "email":"maxsong@example.com", "sub":"fictional-person", "type":"app",
        "nbf":now - 60, "exp":now + 3600})
}

pub(super) fn signed(claims: &Value) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("test-key".into());
    token(&header, claims)
}

pub(super) fn token(header: &Header, claims: &Value) -> String {
    encode(
        header,
        claims,
        &EncodingKey::from_rsa_pem(PRIVATE_KEY).unwrap(),
    )
    .unwrap()
}

pub(super) fn jwks(kid: &str) -> Value {
    let encoding = EncodingKey::from_rsa_pem(PRIVATE_KEY).unwrap();
    let key = jsonwebtoken::jwk::Jwk::from_encoding_key(&encoding, Algorithm::RS256).unwrap();
    let mut key = serde_json::to_value(key).unwrap();
    key["kid"] = kid.into();
    key["alg"] = "RS256".into();
    key["use"] = "sig".into();
    json!({"keys":[key]})
}

pub(super) struct MemorySource {
    pub(super) body: Mutex<Result<Vec<u8>, AccessError>>,
    pub(super) calls: AtomicUsize,
}

impl MemorySource {
    pub(super) fn new(kid: &str) -> Arc<Self> {
        Arc::new(Self {
            body: Mutex::new(Ok(serde_json::to_vec(&jwks(kid)).unwrap())),
            calls: AtomicUsize::new(0),
        })
    }

    pub(super) fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl KeySource for MemorySource {
    fn fetch(&self) -> Fetch {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let result = self.body.lock().unwrap().clone();
        Box::pin(async move {
            tokio::task::yield_now().await;
            result
        })
    }
}

/// 可挂起的取钥替身：启动时的首次取钥立即返回 `initial` 的 JWKS；之后每次取钥都挂起，
/// 直到 [`GatedSource::release`] 放行才返回 `rotated` 的 JWKS，用来稳定制造「取钥在途」窗口。
pub(super) struct GatedSource {
    initial: Vec<u8>,
    rotated: Vec<u8>,
    calls: AtomicUsize,
    open: watch::Sender<bool>,
}

impl GatedSource {
    pub(super) fn new(initial: &str, rotated: &str) -> Arc<Self> {
        Arc::new(Self {
            initial: serde_json::to_vec(&jwks(initial)).unwrap(),
            rotated: serde_json::to_vec(&jwks(rotated)).unwrap(),
            calls: AtomicUsize::new(0),
            open: watch::Sender::new(false),
        })
    }

    pub(super) fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// 放行在途与之后的全部取钥。
    pub(super) fn release(&self) {
        self.open.send_replace(true);
    }

    /// 让出调度，直到累计取钥次数达到 `calls`，即第 `calls` 次取钥已经在途。
    pub(super) async fn wait_for_calls(&self, calls: usize) {
        while self.count() < calls {
            tokio::task::yield_now().await;
        }
    }
}

impl KeySource for GatedSource {
    fn fetch(&self) -> Fetch {
        let startup = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        let body = if startup {
            self.initial.clone()
        } else {
            self.rotated.clone()
        };
        let mut open = self.open.subscribe();
        Box::pin(async move {
            if !startup && open.wait_for(|open| *open).await.is_err() {
                return Err(AccessError::Unavailable);
            }
            Ok(body)
        })
    }
}

pub(super) async fn verifier() -> AccessVerifier {
    AccessVerifier {
        config: config(),
        keys: Arc::new(
            KeyCache::connect(MemorySource::new("test-key"))
                .await
                .unwrap(),
        ),
    }
}
