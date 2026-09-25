//! 静态资源缓存头：只有 `/assets/` 下带 Vite 内容哈希、以对应 MIME 成功送出的文件本体长期缓存，
//! 其余一律 no-store。由 `security::guard` 在定完 CSP、状态与 HEAD 之后调用。

use axum::http::HeaderValue;

/// 内容一变文件名就变，旧名永远对应同一份字节，所以可长期缓存且无需再验证。
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const NO_STORE: &str = "no-store";

/// 缓存策略：`/assets/` 下文件名带 Vite 内容哈希（`名字-哈希.js|css|svg|woff2`）、且确以对应
/// MIME 送出的脚本、样式、SVG 图标与 woff2 字体长期缓存；index.html 每次带不同 CSP nonce，
/// 与 API、错误及其余响应一律 no-store。四类共用同一条哈希判定与同一张 MIME 表。
pub(super) fn cache_control_for(path: &str, content_type: Option<&HeaderValue>) -> HeaderValue {
    let expected = match path
        .strip_prefix("/assets/")
        .and_then(|name| name.rsplit_once('.'))
    {
        Some((stem, "js")) if content_hashed(stem) => crate::assets::JAVASCRIPT,
        Some((stem, "css")) if content_hashed(stem) => crate::assets::STYLESHEET,
        Some((stem, "svg")) if content_hashed(stem) => crate::assets::SVG,
        Some((stem, "woff2")) if content_hashed(stem) => crate::assets::WOFF2,
        _ => return HeaderValue::from_static(NO_STORE),
    };
    if content_type.is_some_and(|value| value == expected) {
        HeaderValue::from_static(IMMUTABLE)
    } else {
        HeaderValue::from_static(NO_STORE)
    }
}

/// Vite 默认产物名 `[name]-[hash]` 的哈希段长度：rolldown `[hash]` 不带长度时取 8 位。
const HASH_LEN: usize = 8;

/// Vite 产物名 `[name]-[hash]`：rolldown 默认 `hashCharacters: 'base64'`，即 URL 安全字母表
/// `[A-Za-z0-9_-]`。只认最后一个路径段恰为「非空名字 + `-` + 恰好 8 位该字母表字符」；哈希本身
/// 也可能含 `-`，所以按距末尾的固定位置切，不找「第一个」或「最后一个」`-`。
/// 多段连字符的普通名（`archivo-latin-800-normal`、`some-icon`）因此不算带哈希；仍会误认的只有
/// 末段恰好 8 位且前面紧跟 `-` 的名字（如 `Inter-SemiBold`），这类文件不得放进 `/assets/`。
fn content_hashed(stem: &str) -> bool {
    let file = stem
        .rsplit_once('/')
        .map_or(stem, |(_, file)| file)
        .as_bytes();
    let Some(dash) = file.len().checked_sub(HASH_LEN + 1) else {
        return false;
    };
    dash > 0
        && file[dash] == b'-'
        && file[dash + 1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 经真实路由只拿得到「类型与扩展名一致」的 200，类型不符的分支只能在这里直接锁住：
    /// 四类长缓存资源各自只认资产处理器给自己的那个 MIME；别类的、缺失或 HTML 都是 no-store。
    #[test]
    fn each_long_cached_type_accepts_only_its_own_mime() {
        let cases = [
            ("/assets/index-C65IIHot.js", crate::assets::JAVASCRIPT),
            ("/assets/index-ovxLtHW2.css", crate::assets::STYLESHEET),
            ("/assets/favicon-4NhN4y5N.svg", crate::assets::SVG),
            (
                "/assets/archivo-latin-800-normal-DZa_k145.woff2",
                crate::assets::WOFF2,
            ),
        ];
        let html = HeaderValue::from_static("text/html; charset=utf-8");
        for (path, own) in cases {
            let own = HeaderValue::from_static(own);
            assert_eq!(cache_control_for(path, Some(&own)), IMMUTABLE, "{path}");
            assert_eq!(cache_control_for(path, None), NO_STORE, "{path}");
            assert_eq!(cache_control_for(path, Some(&html)), NO_STORE, "{path}");
            for (_, other) in cases {
                let other = HeaderValue::from_static(other);
                if other != own {
                    assert_eq!(cache_control_for(path, Some(&other)), NO_STORE, "{path}");
                }
            }
        }
    }

    /// 正例取自真实 `web/dist/assets` 产物名（哈希可以 `_` 或 `-` 开头、中间含 `-`，名字可多段
    /// 连字符或在子目录）；反例是多段连字符的普通名、长度不是 8 的哈希段、空名字与字母表外字符。
    #[test]
    fn content_hashed_accepts_only_a_name_dash_and_exactly_eight_base64url_chars() {
        for stem in [
            "index-C65IIHot",
            "index-_njak0q8",
            "MarkdownEditor-BqsrZiZp",
            "MarkdownEditor-CC7-Ql5Q",
            "x--bcdEfGh",
            "favicon-4NhN4y5N",
            "archivo-latin-800-normal-DZa_k145",
            "ibm-plex-mono-latin-400-normal-DMJ8VG8y",
            "chunks/vendor-Abcdefgh",
        ] {
            assert!(content_hashed(stem), "{stem}");
        }
        for stem in [
            "archivo-latin-800-normal",
            "ibm-plex-mono-latin-400-normal",
            "some-icon",
            "my-long-icon",
            "favicon",
            "favicon-fixture",
            "font-Abcdefg",
            "vendor-Abcdefgh12",
            "vendor-Abcdefgh.min",
            "-Abcdefgh",
            "chunks/-Abcdefgh",
            "Abcdefgh",
            "a-éééé",
            "index-汉字汉",
            "",
        ] {
            assert!(!content_hashed(stem), "{stem}");
        }
    }

    /// 类型与扩展名一致也不够：没有 Vite 哈希段的多段连字符名一律 no-store，改内容后浏览器才会重取。
    #[test]
    fn unhashed_multi_dash_names_stay_no_store_even_with_their_own_mime() {
        for (path, own) in [
            (
                "/assets/archivo-latin-800-normal.woff2",
                crate::assets::WOFF2,
            ),
            (
                "/assets/ibm-plex-mono-latin-400-normal.woff2",
                crate::assets::WOFF2,
            ),
            ("/assets/some-icon.svg", crate::assets::SVG),
            ("/assets/my-long-icon.svg", crate::assets::SVG),
            (
                "/assets/chunks/vendor-Abcdefgh12.js",
                crate::assets::JAVASCRIPT,
            ),
            ("/assets/code-mirror-theme.css", crate::assets::STYLESHEET),
        ] {
            let own = HeaderValue::from_static(own);
            assert_eq!(cache_control_for(path, Some(&own)), NO_STORE, "{path}");
        }
    }
}
