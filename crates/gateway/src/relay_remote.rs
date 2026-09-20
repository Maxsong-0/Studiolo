//! Credential-free front transport and separate-UID key-holder framing.
use crate::{HttpsRelayTransport, RelayError, RelayGrant, RelayTransport};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    grant: RelayGrant,
    request: Vec<u8>,
}
fn peer(stream: &UnixStream, expected_uid: u32) -> Result<(), RelayError> {
    #[cfg(target_os = "linux")]
    {
        let peer = rustix::net::sockopt::socket_peercred(stream).map_err(|_| RelayError::Denied)?;
        if expected_uid == 0
            || rustix::process::geteuid().as_raw() == 0
            || peer.uid.as_raw() != expected_uid
            || expected_uid == rustix::process::geteuid().as_raw()
        {
            return Err(RelayError::Denied);
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (stream, expected_uid);
        Err(RelayError::Denied)
    }
}
fn read_frame(stream: &mut UnixStream, ceiling: usize) -> Result<Vec<u8>, RelayError> {
    let mut size = [0u8; 4];
    stream
        .read_exact(&mut size)
        .map_err(|_| RelayError::Upstream)?;
    let size = u32::from_be_bytes(size) as usize;
    if size > ceiling {
        return Err(RelayError::Upstream);
    }
    let mut bytes = vec![0; size];
    stream
        .read_exact(&mut bytes)
        .map_err(|_| RelayError::Upstream)?;
    Ok(bytes)
}
fn write_frame(stream: &mut UnixStream, bytes: &[u8]) -> Result<(), RelayError> {
    let size = u32::try_from(bytes.len())
        .map_err(|_| RelayError::Upstream)?
        .to_be_bytes();
    stream
        .write_all(&size)
        .and_then(|()| stream.write_all(bytes))
        .and_then(|()| stream.flush())
        .map_err(|_| RelayError::Upstream)
}
/// Front-process transport which has no API keys and never reads their files.
pub struct RemoteRelayTransport {
    socket: PathBuf,
    gateway_uid: u32,
}
impl RemoteRelayTransport {
    /// Pins the key-holder's independent OS identity and private Unix endpoint.
    #[must_use]
    pub fn new(socket: PathBuf, gateway_uid: u32) -> Self {
        Self {
            socket,
            gateway_uid,
        }
    }
    /// Verifies the live key-holder peer UID without loading or requesting any credential.
    pub fn probe(&self) -> Result<(), RelayError> {
        let stream = UnixStream::connect(&self.socket).map_err(|_| RelayError::Upstream)?;
        peer(&stream, self.gateway_uid)
    }
}
impl RelayTransport for RemoteRelayTransport {
    fn execute(
        &self,
        grant: &RelayGrant,
        request: &[u8],
        check: &dyn Fn() -> Result<(), RelayError>,
    ) -> Result<Vec<u8>, RelayError> {
        check()?;
        let mut stream = UnixStream::connect(&self.socket).map_err(|_| RelayError::Upstream)?;
        peer(&stream, self.gateway_uid)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(31)))
            .map_err(|_| RelayError::Upstream)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .map_err(|_| RelayError::Upstream)?;
        let request = serde_json::to_vec(&Envelope {
            grant: grant.clone(),
            request: request.to_vec(),
        })
        .map_err(|_| RelayError::Request)?;
        if request.len() > 24 * 1024 * 1024 {
            return Err(RelayError::Request);
        }
        write_frame(&mut stream, &request)?;
        let mut output = Vec::new();
        loop {
            check()?;
            let chunk = read_frame(&mut stream, grant.limits.response_bytes as usize)?;
            if chunk.is_empty() {
                break;
            }
            if output.len().saturating_add(chunk.len()) > grant.limits.response_bytes as usize {
                return Err(RelayError::Upstream);
            }
            output.extend_from_slice(&chunk);
        }
        check()?;
        crate::relay_transport::validate_terminal(grant.provider, &output)?;
        Ok(output)
    }
}
/// Serves one request only from the pinned independent front-process UID.
/// Frames remain internal; the front rechecks authority before consuming every chunk.
pub fn serve_key_holder_connection(
    transport: &HttpsRelayTransport,
    mut stream: UnixStream,
    client_uid: u32,
) -> Result<(), RelayError> {
    peer(&stream, client_uid)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| RelayError::Upstream)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| RelayError::Upstream)?;
    let bytes = read_frame(&mut stream, 24 * 1024 * 1024)?;
    let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|_| RelayError::Request)?;
    if envelope.grant.limits.response_bytes > 16 * 1024 * 1024
        || envelope.grant.limits.timeout_seconds > 300
        || envelope.grant.limits.timeout_seconds == 0
    {
        return Err(RelayError::Request);
    }
    // Defence in depth: even the trusted front cannot redirect the key-holder to an arbitrary URL.
    let request = crate::relay_request::normalize(&envelope.grant, &envelope.request)?;
    transport.forward(&envelope.grant, &request, &|| Ok(()), &mut |chunk| {
        write_frame(&mut stream, chunk)
    })?;
    write_frame(&mut stream, &[])
}
