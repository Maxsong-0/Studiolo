//! Exact call inventories and boundary anchors for the final M6 implementation.
use std::path::Path;
pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo" {
        return source.to_owned();
    }
    if path.ends_with("crates/studiolo/src/demo_gateway.rs") {
        return super::authority_tokens::checked_closed(
            source,
            &[
                "config.mode!=\"demo\"",
                "!config.endpoint.ip().is_loopback()",
                "config.endpoint.port()==0",
                ".is_none_or(|n|n!=\"demo.sqlite3\")",
                "PRAGMAquery_only=ON;PRAGMAtrusted_schema=OFF;",
                "GET/demo.jsonHTTP/1.1",
                "self.admission.invoke(cap,call.clone())?;",
                "ifheaders.len()>8192",
                ".filter(|n|*n<=65536)",
            ],
            &[
                (".write_all(request.as_bytes())", 1),
                (
                    "TcpStream::connect_timeout(&self.endpoint,Duration::from_secs(2))",
                    1,
                ),
                (
                    "Connection::open_with_flags(&config.database,OpenFlags::SQLITE_OPEN_READ_ONLY|OpenFlags::SQLITE_OPEN_NO_MUTEX)",
                    1,
                ),
            ],
        );
    }
    source.to_owned()
}
