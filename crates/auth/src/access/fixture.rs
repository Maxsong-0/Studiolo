//! 非默认测试 Adapter：只有固定虚构配置和公钥，不含签名材料或通用工厂。

use std::sync::Arc;

use super::{
    AccessConfig, AccessError, AccessVerifier,
    keys::{Fetch, KeyCache, KeySource},
};

// 公钥来自 jsonwebtoken 10.3.0 的 MIT 测试夹具；签名材料不在此编译单元中。
const PUBLIC_JWKS: &[u8] = br#"{"keys":[{"kid":"test-key","kty":"RSA","alg":"RS256","use":"sig","n":"yRE6rHuNR0QbHO3H3Kt2pOKGVhQqGZXInOduQNxXzuKlvQTLUTv4l4sggh5_CYYi_cvI-SXVT9kPWSKXxJXBXd_4LkvcPuUakBoAkfh-eiFVMh2VrUyWyj3MFl0HTVF9KwRXLAcwkREiS3npThHRyIxuy0ZMeZfxVL5arMhw1SRELB8HoGfG_AtH89BIE9jDBHZ9dLelK9a184zAf8LwoPLxvJb3Il5nncqPcSfKDDodMFBIMc4lQzDKL5gvmiXLXB1AGLm8KBjfE8s3L5xqi-yUod-j8MtvIj812dkS4QMiRVN_by2h3ZY8LYVGrqZXZTcgn2ujn8uKjXLZVD5TdQ","e":"AQAB"}]}"#;

struct FixedFixtureSource;

impl KeySource for FixedFixtureSource {
    fn fetch(&self) -> Fetch {
        Box::pin(async { Ok(PUBLIC_JWKS.to_vec()) })
    }
}

/// 创建只接受固定虚构配置的测试验证器，不联网且仍执行真实 RS256 校验。
///
/// 仅非默认 `test-support` feature 导出；生产构建不得启用。此函数不接受配置、
/// JWKS 或主体参数，不能用于真实 Access 应用，也不会签发身份或 JWT。
pub async fn fixture_verifier() -> Result<AccessVerifier, AccessError> {
    Ok(AccessVerifier {
        config: AccessConfig::new(
            "https://studiolo.example.com".into(),
            "https://studiolo.cloudflareaccess.com".into(),
            "example-audience".into(),
            "maxsong@example.com".into(),
        )?,
        keys: Arc::new(KeyCache::connect(Arc::new(FixedFixtureSource)).await?),
    })
}
