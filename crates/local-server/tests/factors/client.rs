//! 真实Unix HTTP/1.1客户端；手工Cookie仅验证协议，不冒称浏览器Secure Cookie验收。

use axum::http::{HeaderMap, HeaderName, HeaderValue, Request};
use std::{path::Path, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

pub struct Client {
    stream: UnixStream,
    buffered: Vec<u8>,
}

pub struct Reply {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Client {
    pub async fn connect(path: &Path) -> Self {
        Self {
            stream: UnixStream::connect(path).await.unwrap(),
            buffered: Vec::new(),
        }
    }

    pub async fn write(&mut self, request: Request<Vec<u8>>) {
        let (mut parts, body) = request.into_parts();
        if !parts.headers.contains_key("content-length")
            && !parts.headers.contains_key("transfer-encoding")
        {
            parts
                .headers
                .insert("content-length", body.len().to_string().parse().unwrap());
        }
        let mut encoded = format!("{} {} HTTP/1.1\r\n", parts.method, parts.uri).into_bytes();
        for (name, value) in &parts.headers {
            encoded.extend_from_slice(name.as_str().as_bytes());
            encoded.extend_from_slice(b": ");
            encoded.extend_from_slice(value.as_bytes());
            encoded.extend_from_slice(b"\r\n");
        }
        encoded.extend_from_slice(b"\r\n");
        encoded.extend_from_slice(&body);
        self.stream.write_all(&encoded).await.unwrap();
    }

    pub async fn send(&mut self, request: Request<Vec<u8>>) -> Reply {
        let head = request.method() == "HEAD";
        self.write(request).await;
        tokio::time::timeout(Duration::from_secs(6), self.read(head))
            .await
            .unwrap()
    }

    async fn read(&mut self, head: bool) -> Reply {
        let end = loop {
            if let Some(position) = self
                .buffered
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
            {
                break position;
            }
            self.more().await;
            assert!(self.buffered.len() <= 32 * 1024);
        };
        let header = std::str::from_utf8(&self.buffered[..end]).unwrap();
        let mut lines = header.split("\r\n");
        let status = lines
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let mut headers = HeaderMap::new();
        for line in lines {
            let (name, value) = line.split_once(':').unwrap();
            headers.append(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value.trim()).unwrap(),
            );
        }
        assert!(
            !headers.contains_key("transfer-encoding"),
            "固定因子响应应具有确定长度"
        );
        let length = if head || status == 204 {
            0
        } else {
            headers
                .get("content-length")
                .map_or(0, |value| value.to_str().unwrap().parse::<usize>().unwrap())
        };
        assert!(length <= 16 * 1024);
        self.buffered.drain(..end + 4);
        while self.buffered.len() < length {
            self.more().await;
        }
        let body = self.buffered.drain(..length).collect();
        Reply {
            status,
            headers,
            body,
        }
    }

    async fn more(&mut self) {
        let mut bytes = [0_u8; 4096];
        let length = self.stream.read(&mut bytes).await.unwrap();
        assert_ne!(length, 0, "响应提前结束");
        self.buffered.extend_from_slice(&bytes[..length]);
    }
}

impl Reply {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap()
    }

    pub fn secure(&self) {
        for (name, expected) in [
            ("cache-control", "no-store"),
            ("x-content-type-options", "nosniff"),
            ("referrer-policy", "no-referrer"),
            ("cross-origin-resource-policy", "same-origin"),
            ("vary", "Origin, Sec-Fetch-Site"),
        ] {
            assert_eq!(self.headers[name], expected);
        }
        assert!(
            self.headers["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("default-src 'none'")
        );
        assert!(
            !self
                .headers
                .keys()
                .any(|name| name.as_str().starts_with("access-control-"))
        );
    }

    pub fn kept_cookie(&self) {
        assert!(!self.headers.contains_key("set-cookie"));
    }

    pub fn cleared_cookie(&self) {
        let cookie = self.headers["set-cookie"].to_str().unwrap();
        assert!(cookie.starts_with("__Host-studiolo-session="));
        for value in [
            "Secure",
            "HttpOnly",
            "SameSite=Strict",
            "Path=/",
            "Max-Age=0",
        ] {
            assert!(cookie.contains(value));
        }
    }
}
