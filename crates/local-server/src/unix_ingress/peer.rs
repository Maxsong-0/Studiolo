//! 凭据只由真实连接构造；来源 IP 必须逐请求解析，不能缓存在连接上。

use super::IngressError;
use axum::{
    extract::{ConnectInfo, Request, connect_info::Connected},
    http::HeaderMap,
    serve::IncomingStream,
};
use rustix::process::{getegid, geteuid, getgid, getuid};
use std::net::IpAddr;
use tokio::net::UnixListener;

#[derive(Clone)]
pub(super) struct UnixPeer {
    uid: Option<u32>,
}

impl Connected<IncomingStream<'_, UnixListener>> for UnixPeer {
    fn connect_info(stream: IncomingStream<'_, UnixListener>) -> Self {
        Self {
            uid: stream.io().peer_cred().ok().map(|cred| cred.uid()),
        }
    }
}

/// 字段私有；调用方只能读取经来源验证的本次请求 IP。
pub(crate) struct TrustedNetworkSource {
    ip: IpAddr,
}

impl TrustedNetworkSource {
    pub(crate) fn ip(&self) -> IpAddr {
        self.ip
    }
}

pub(crate) fn source(request: &Request) -> Result<TrustedNetworkSource, IngressError> {
    // 不使用 ConnectInfo extractor 的 MockConnectInfo 后备，也不接受 TCP extension。
    let peer = request
        .extensions()
        .get::<ConnectInfo<UnixPeer>>()
        .ok_or(IngressError::UntrustedSource)?;
    if getuid() != geteuid() || getgid() != getegid() || !same_uid(peer.0.uid, getuid().as_raw()) {
        return Err(IngressError::UntrustedSource);
    }
    Ok(TrustedNetworkSource {
        ip: request_ip(request.headers())?,
    })
}

fn same_uid(peer: Option<u32>, own: u32) -> bool {
    peer == Some(own)
}

fn request_ip(headers: &HeaderMap) -> Result<IpAddr, IngressError> {
    let mut values = headers.get_all("cf-connecting-ip").iter();
    let value = values
        .next()
        .ok_or(IngressError::UntrustedSource)?
        .as_bytes();
    if values.next().is_some()
        || value.is_empty()
        || value.len() > 45
        || value.iter().any(|byte| {
            !byte.is_ascii()
                || byte.is_ascii_whitespace()
                || matches!(*byte, b',' | b'%' | b'[' | b']')
        })
    {
        return Err(IngressError::UntrustedSource);
    }
    std::str::from_utf8(value)
        .map_err(|_| IngressError::UntrustedSource)?
        .parse()
        .map_err(|_| IngressError::UntrustedSource)
}

#[cfg(test)]
#[path = "../../tests/unix_ingress/headers.rs"]
mod tests;
