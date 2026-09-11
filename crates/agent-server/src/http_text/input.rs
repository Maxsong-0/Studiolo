//! 有界且严格的表单查询解码：拒绝重复、未知字段及不合法 UTF-8，不回显输入。
use axum::http::HeaderMap;
use std::collections::BTreeMap;

pub(super) const SESSION_HEADER: &str = "studiolo-session-id";
pub(super) const INSTANCE_HEADER: &str = "studiolo-instance-id";
const MAX_QUERY_BYTES: usize = 8192;

pub(super) fn session_id(headers: &HeaderMap) -> Result<Option<String>, ()> {
    let mut values = headers.get_all(SESSION_HEADER).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(());
    }
    let id = value.to_str().map_err(|_| ())?;
    if id.len() != 26 || ulid::Ulid::from_string(id).map_err(|_| ())?.to_string() != id {
        return Err(());
    }
    Ok(Some(id.to_owned()))
}

pub(super) fn query(
    raw: Option<&str>,
    allowed: &[&str],
    required: &[&str],
) -> Result<BTreeMap<String, String>, ()> {
    let raw = raw.unwrap_or("");
    if raw.len() > MAX_QUERY_BYTES {
        return Err(());
    }
    let mut values = BTreeMap::new();
    if !raw.is_empty() {
        for pair in raw.split('&') {
            let (key, value) = pair.split_once('=').ok_or(())?;
            let key = decode(key)?;
            let value = decode(value)?;
            if !allowed.contains(&key.as_str()) || values.insert(key, value).is_some() {
                return Err(());
            }
        }
    }
    if required
        .iter()
        .any(|key| values.get(*key).is_none_or(String::is_empty))
    {
        return Err(());
    }
    Ok(values)
}

fn decode(raw: &str) -> Result<String, ()> {
    let mut bytes = raw.bytes();
    let mut out = Vec::with_capacity(raw.len());
    while let Some(byte) = bytes.next() {
        out.push(match byte {
            b'+' => b' ',
            b'%' => {
                let hi = char::from(bytes.next().ok_or(())?).to_digit(16).ok_or(())?;
                let lo = char::from(bytes.next().ok_or(())?).to_digit(16).ok_or(())?;
                (hi * 16 + lo) as u8
            }
            other => other,
        });
    }
    let decoded = String::from_utf8(out).map_err(|_| ())?;
    if decoded.chars().any(char::is_control) {
        return Err(());
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn form_decoding_is_exact_and_rejects_ambiguous_or_lossy_input() {
        assert_eq!(
            query(
                Some("path=%E4%B8%AD%E6%96%87%2Fnote%2B1.md"),
                &["path"],
                &["path"]
            )
            .unwrap()["path"],
            "中文/note+1.md"
        );
        for value in [
            "path=x&path=y",
            "path=x&%70ath=y",
            "token=x",
            "path=%ff",
            "path=%",
            "path=%0G",
            "path=%00",
            "path=",
            "path=x&",
            "path",
        ] {
            assert!(query(Some(value), &["path"], &["path"]).is_err(), "{value}");
        }
        assert!(
            query(
                Some(&format!("path={}", "x".repeat(MAX_QUERY_BYTES))),
                &["path"],
                &["path"]
            )
            .is_err()
        );
        assert!(query(None, &["path"], &["path"]).is_err());
        assert!(query(None, &[], &[]).is_ok());
    }
}
