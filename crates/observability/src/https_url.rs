//! 出站地址的形状检查：告警里的可选链接、门铃的收件地址和链接共用。纯逻辑，没有 IO。

/// 出站地址的长度上限。
const MAX_BYTES: usize = 2048;

/// 只收 HTTPS、有主机名、不带用户名口令和片段的地址；`allow_query` 决定能不能带查询串。
///
/// 空白、控制字符、反斜杠一律拒绝。URL 解析会消去空的 userinfo，也会把 `https:///host`
/// 里多出来的斜杠吃掉、拿路径第一段当主机，所以另查原始 authority：为空或带 `@` 都拒绝。
pub(crate) fn checked(value: &str, allow_query: bool) -> Option<reqwest::Url> {
    if value.len() > MAX_BYTES
        || value
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
        || value.contains('\\')
    {
        return None;
    }
    let url = reqwest::Url::parse(value).ok()?;
    let authority = value
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or_default())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || authority.is_empty()
        || authority.contains('@')
        || !url.username().is_empty()
        || url.password().is_some()
        || (!allow_query && url.query().is_some())
        || url.fragment().is_some()
    {
        return None;
    }
    Some(url)
}

#[cfg(test)]
mod tests {
    use super::checked;

    #[test]
    fn only_plain_https_addresses_pass() {
        assert!(checked("https://example.test/hook", false).is_some());
        assert!(checked("https://example.test/hook?route=a", true).is_some());
        for refused in [
            "http://example.test/hook",
            "https://user@example.test/hook",
            "https://user:pass@example.test/hook",
            "https://@example.test/hook",
            "https://example.test/hook#part",
            "https://example.test/ho ok",
            "https://example.test\\hook",
            "https:///hook",
            "https://",
            "https:example.test/hook",
            "ftp://example.test/hook",
            "example.test/hook",
            "",
        ] {
            assert!(checked(refused, true).is_none(), "不该接受：{refused:?}");
        }
        assert!(checked("https://example.test/hook?route=a", false).is_none());
        let long = format!("https://example.test/{}", "a".repeat(2048));
        assert!(checked(&long, true).is_none());
    }
}
