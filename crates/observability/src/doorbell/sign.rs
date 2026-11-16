//! Standard Webhooks 1.0.0 的对称签名（`v1`＝HMAC-SHA256）。纯计算，没有 IO。
//!
//! 被签的内容是 `消息编号.时间戳.正文`，密钥是 `whsec_` 后面那段 base64 解出来的字节。
//! 规范：<https://github.com/standard-webhooks/standard-webhooks/blob/main/spec/standard-webhooks.md>

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use zeroize::Zeroizing;

const PREFIX: &str = "whsec_";
/// 规范要求密钥在 24 到 64 字节之间。
const KEY_BYTES: std::ops::RangeInclusive<usize> = 24..=64;

/// 签名密钥。不实现 `Debug`／`Display`，离开作用域时清零。
pub struct Secret(Zeroizing<Vec<u8>>);

/// 密钥文本不合规范；不回显任何内容。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("门铃签名密钥不是合法的 whsec_ 密钥")]
pub struct SecretError;

impl Secret {
    /// 解析 `whsec_<base64>`；前缀、编码、长度任何一项不对都拒绝。
    pub fn parse(text: &str) -> Result<Self, SecretError> {
        let encoded = text.strip_prefix(PREFIX).ok_or(SecretError)?;
        let key = Zeroizing::new(STANDARD.decode(encoded).map_err(|_| SecretError)?);
        if !KEY_BYTES.contains(&key.len()) {
            return Err(SecretError);
        }
        Ok(Self(key))
    }
}

/// `webhook-signature` 头的值：`v1,<base64(HMAC-SHA256)>`。
///
/// HMAC 接受任意长度的密钥，正常不会失败；真失败了就返回错误，调用方不得发出没签名的请求。
pub fn signature(
    secret: &Secret,
    message_id: &str,
    timestamp_secs: i64,
    body: &[u8],
) -> Result<String, SecretError> {
    let mut mac =
        Hmac::<sha2_legacy::Sha256>::new_from_slice(&secret.0).map_err(|_| SecretError)?;
    mac.update(message_id.as_bytes());
    mac.update(b".");
    mac.update(timestamp_secs.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    Ok(format!(
        "v1,{}",
        STANDARD.encode(mac.finalize().into_bytes())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 公开文档里的示例向量（Svix「手动验证 Webhook」一节），不是任何真实密钥。
    #[test]
    fn the_published_example_vector_matches() {
        let secret = Secret::parse("whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw").unwrap();
        assert_eq!(
            signature(
                &secret,
                "msg_p5jXN8AQM9LWM0D4loKWxJek",
                1_614_265_330,
                br#"{"test": 2432232314}"#
            )
            .unwrap(),
            "v1,g0hM9SsE+OTPJTGt/tmIKtSyZlE3uFJELVlNIOLJ1OE="
        );
    }

    #[test]
    fn any_change_to_id_time_or_body_changes_the_signature() {
        let secret = Secret::parse("whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw").unwrap();
        let sign = |id, time, body| signature(&secret, id, time, body).unwrap();
        let base = sign("msg_a", 10, b"{}".as_slice());
        assert_ne!(base, sign("msg_b", 10, b"{}"));
        assert_ne!(base, sign("msg_a", 11, b"{}"));
        assert_ne!(base, sign("msg_a", 10, b"{ }"));
        assert_eq!(base, sign("msg_a", 10, b"{}"));
    }

    #[test]
    fn malformed_secrets_are_refused() {
        for text in [
            "",
            "MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw",
            "whsec_",
            "whsec_not base64!",
            // 解出来只有 3 字节，低于规范下限。
            "whsec_AAAA",
        ] {
            assert!(Secret::parse(text).is_err(), "不该接受：{text:?}");
        }
        // 65 字节，高于规范上限。
        let long = format!("whsec_{}", STANDARD.encode([7_u8; 65]));
        assert!(Secret::parse(&long).is_err());
    }
}
