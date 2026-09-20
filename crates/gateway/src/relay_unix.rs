//! A deliberately small HTTP/1.1 Unix socket boundary; no TCP listener or user-selected target.
#[cfg(unix)]
use crate::{CallId, DurableRelay, RelayError};
#[cfg(unix)]
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    time::Duration,
};

/// Serves one bounded request on an already accepted, protected Unix connection.
/// The owner must bind the listener in a Gateway-owned 0700 directory and bound concurrency.
#[cfg(unix)]
pub fn serve_relay_connection(
    relay: &DurableRelay,
    mut stream: UnixStream,
) -> Result<(), RelayError> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| RelayError::Request)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| RelayError::Request)?;
    let result = read_request(&mut stream)
        .and_then(|(ticket, call, path, body)| relay.relay(&ticket, call, &path, &body));
    let (status, content_type, body) = match result {
        Ok(result) => (200, result.content_type, result.body),
        Err(error) => {
            let status = match error {
                RelayError::Budget => 429,
                RelayError::Frozen
                | RelayError::Storage
                | RelayError::Audit
                | RelayError::Publication => 503,
                RelayError::Indeterminate | RelayError::Conflict | RelayError::Expired => 409,
                _ => 403,
            };
            (
                status,
                "application/json",
                serde_json::to_vec(&serde_json::json!({"error":error.to_string()}))
                    .map_err(|_| RelayError::Storage)?,
            )
        }
    };
    write!(stream, "HTTP/1.1 {status} Relay\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())
        .and_then(|()| stream.write_all(&body)).and_then(|()| stream.flush()).map_err(|_| RelayError::Request)
}
#[cfg(unix)]
fn read_request(stream: &mut UnixStream) -> Result<(String, CallId, String, Vec<u8>), RelayError> {
    let mut reader = BufReader::new(stream);
    let mut header = Vec::new();
    loop {
        // BufRead::read_until alone is unbounded; take caps each read and the whole header.
        let read = reader
            .by_ref()
            .take((16385 - header.len()) as u64)
            .read_until(b'\n', &mut header)
            .map_err(|_| RelayError::Request)?;
        if read == 0 || header.len() > 16384 {
            return Err(RelayError::Request);
        }
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let text = std::str::from_utf8(&header).map_err(|_| RelayError::Request)?;
    let mut lines = text.split("\r\n");
    let request: Vec<_> = lines
        .next()
        .ok_or(RelayError::Request)?
        .split(' ')
        .collect();
    if request.len() != 3
        || request[0] != "POST"
        || request[2] != "HTTP/1.1"
        || !matches!(request[1], "/v1/messages" | "/v1/responses")
    {
        return Err(RelayError::Request);
    }
    let path = request[1].to_owned();
    let (mut ticket, mut length, mut call) = (None, None, None);
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or(RelayError::Request)?;
        match name.to_ascii_lowercase().as_str() {
            "authorization" if ticket.is_none() => {
                ticket = Some(
                    value
                        .trim()
                        .strip_prefix("Bearer ")
                        .ok_or(RelayError::Denied)?
                        .to_owned(),
                )
            }
            "content-length" if length.is_none() => {
                length = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| RelayError::Request)?,
                )
            }
            "x-studiolo-call-id" if call.is_none() => {
                call = Some(CallId::parse(value.trim()).map_err(|_| RelayError::Request)?)
            }
            "authorization" | "content-length" | "x-studiolo-call-id" | "transfer-encoding" => {
                return Err(RelayError::Request);
            }
            _ => {}
        }
    }
    let length = length
        .filter(|n| *n > 0 && *n <= 4 * 1024 * 1024)
        .ok_or(RelayError::Request)?;
    let mut body = vec![0; length];
    reader
        .read_exact(&mut body)
        .map_err(|_| RelayError::Request)?;
    Ok((
        ticket.ok_or(RelayError::Denied)?,
        call.ok_or(RelayError::Request)?,
        path,
        body,
    ))
}
