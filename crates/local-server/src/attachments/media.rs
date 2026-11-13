//! 扩展名仅选择校验规则；内容校验失败一律降为仅下载类型。

pub(super) fn content_type(path: &str, bytes: &[u8]) -> &'static str {
    let extension = path
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase());
    match extension.as_deref() {
        Some("png") if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => "image/png",
        Some("jpg" | "jpeg") if bytes.starts_with(b"\xff\xd8\xff") => "image/jpeg",
        Some("gif") if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") => "image/gif",
        Some("webp") if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") => {
            "image/webp"
        }
        Some("pdf") if bytes.starts_with(b"%PDF-") => "application/pdf",
        Some("txt" | "csv" | "tsv" | "json" | "canvas" | "yaml" | "yml" | "toml" | "log")
            if !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok() =>
        {
            "text/plain; charset=utf-8"
        }
        _ => "application/octet-stream",
    }
}

pub(super) fn disposition(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or("attachment");
    let fallback: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    // 编码所有非字母数字与安全标点，头值中不会出现引号、控制字符或非 ASCII。
    let encoded: String = name
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-') {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

#[cfg(test)]
mod tests {
    use super::content_type;

    #[test]
    fn every_text_extension_requires_utf8_without_nul_and_active_types_never_preview() {
        for extension in [
            "txt", "csv", "tsv", "json", "canvas", "yaml", "yml", "toml", "log",
        ] {
            let path = format!("file.{extension}");
            assert_eq!(
                content_type(&path, b"<script>text</script>"),
                "text/plain; charset=utf-8"
            );
            for invalid in [b"\xff".as_slice(), b"a\0b"] {
                assert_eq!(content_type(&path, invalid), "application/octet-stream");
            }
        }
        for extension in [
            "svg", "html", "htm", "xhtml", "svgz", "xml", "js", "mjs", "css",
        ] {
            assert_eq!(
                content_type(&format!("file.{extension}"), b"\x89PNG\r\n\x1a\n"),
                "application/octet-stream"
            );
        }
        assert_eq!(content_type("file.PNG", b"\x89PNG\r\n\x1a\n"), "image/png");
        for (path, bytes) in [
            ("file.png", b"\x89PNG".as_slice()),
            ("file.gif", b"GIF8"),
            ("file.webp", b"RIFFWEBP"),
        ] {
            assert_eq!(content_type(path, bytes), "application/octet-stream");
        }
    }
}
