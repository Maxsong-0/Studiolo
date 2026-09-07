//! 一个 HttpOnly Cookie 原子承载配对秘密；不接受重复应用名或宽松解码。

use axum::http::{HeaderMap, HeaderValue};
use studiolo_auth::{CsrfToken, SessionToken};
use zeroize::Zeroizing;

use super::{gate, responses::Failure};

const NAME: &str = "__Host-studiolo-session";
const ATTRIBUTES: &str = "; Secure; HttpOnly; SameSite=Strict; Path=/";

pub(super) struct Credentials {
    pub token: SessionToken,
    pub csrf: CsrfToken,
}

pub(super) fn value(headers: &HeaderMap) -> Result<Option<&str>, Failure> {
    let Some(encoded) = gate::one(headers, "cookie")? else {
        return Ok(None);
    };
    if encoded.is_empty() || encoded.len() > 8192 {
        return Err(Failure::bad_request());
    }
    let mut found = None;
    let mut count = 0;
    for part in encoded.split(';') {
        count += 1;
        if count > 64 {
            return Err(Failure::bad_request());
        }
        let part = part.trim_matches(' ');
        let (name, value) = part.split_once('=').ok_or_else(Failure::bad_request)?;
        if name.is_empty() || !name.bytes().all(token_character) {
            return Err(Failure::bad_request());
        }
        // 普通 Cookie 可采用规范引号；应用秘密只接受其唯一未加引号编码。
        let unquoted = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value);
        if !unquoted.bytes().all(cookie_character) {
            return Err(Failure::bad_request());
        }
        if name == NAME {
            if found.is_some() || unquoted != value {
                return Err(Failure::bad_request());
            }
            found = Some(value);
        }
    }
    Ok(found)
}

pub(super) fn parse(value: Option<&str>) -> Result<Credentials, Failure> {
    let value = value.ok_or_else(Failure::unauthorized)?;
    if value.len() > 169 {
        return Err(Failure::unauthorized());
    }
    let (token, csrf) = value.split_once('~').ok_or_else(Failure::unauthorized)?;
    Ok(Credentials {
        token: SessionToken::from_hex(token.to_owned()).map_err(|_| Failure::unauthorized())?,
        csrf: CsrfToken::from_hex(csrf.to_owned()).map_err(|_| Failure::unauthorized())?,
    })
}

pub(super) fn issued(token: &SessionToken, csrf: &CsrfToken) -> Result<HeaderValue, Failure> {
    let value = Zeroizing::new(format!(
        "{NAME}={}~{}{ATTRIBUTES}",
        token.expose_secret(),
        csrf.expose_secret()
    ));
    let mut header = HeaderValue::from_str(&value).map_err(|_| Failure::unavailable())?;
    header.set_sensitive(true);
    Ok(header)
}

pub(super) fn expired() -> HeaderValue {
    let mut value = HeaderValue::from_static(
        "__Host-studiolo-session=; Secure; HttpOnly; SameSite=Strict; Path=/; Max-Age=0",
    );
    value.set_sensitive(true);
    value
}

fn token_character(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

fn cookie_character(byte: u8) -> bool {
    matches!(byte, 0x21 | 0x23..=0x2b | 0x2d..=0x3a | 0x3c..=0x5b | 0x5d..=0x7e)
}
