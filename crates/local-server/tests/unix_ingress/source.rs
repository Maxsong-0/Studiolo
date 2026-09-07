//! 私有来源能力以真实 Unix listener 取得；测试不手工构造来源凭据。

#[path = "support.rs"]
mod support;

use super::{PreparedUnixIngress, source};
use axum::{
    Router,
    body::Body,
    extract::{ConnectInfo, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use std::{net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream, UnixStream},
    sync::oneshot,
};

async fn inspect(request: Request) -> Response {
    match source(&request) {
        Ok(proof) => (StatusCode::OK, proof.ip().to_string()).into_response(),
        Err(_) => (StatusCode::FORBIDDEN, "source-rejected").into_response(),
    }
}

fn app() -> Router {
    Router::new().route("/", get(inspect))
}

async fn exchange<S>(stream: &mut BufReader<S>, headers: &str) -> (u16, String)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tokio::time::timeout(Duration::from_secs(5), async {
        stream
            .get_mut()
            .write_all(
                format!("GET / HTTP/1.1\r\nHost: ingress.invalid\r\n{headers}\r\n").as_bytes(),
            )
            .await
            .unwrap();
        let mut line = String::new();
        stream.read_line(&mut line).await.unwrap();
        let status = line.split_whitespace().nth(1).unwrap().parse().unwrap();
        let mut length = None;
        loop {
            line.clear();
            assert_ne!(stream.read_line(&mut line).await.unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = Some(value.trim().parse::<usize>().unwrap());
            }
        }
        let mut body = vec![0; length.unwrap()];
        stream.read_exact(&mut body).await.unwrap();
        (status, String::from_utf8(body).unwrap())
    })
    .await
    .unwrap()
}

#[test]
fn a_request_without_real_unix_connection_proof_is_rejected() {
    let mut request = Request::builder()
        .header("cf-connecting-ip", "192.0.2.1")
        .body(Body::empty())
        .unwrap();
    assert!(source(&request).is_err());
    request
        .extensions_mut()
        .insert(ConnectInfo("127.0.0.1:443".parse::<SocketAddr>().unwrap()));
    assert!(source(&request).is_err());
}

#[tokio::test]
async fn real_same_uid_unix_connection_reads_a_fresh_ip_for_each_request() {
    let fixture = support::Fixture::new();
    let bound = PreparedUnixIngress::prepare(&fixture.socket, &[&fixture.isolated])
        .unwrap()
        .bind()
        .unwrap();
    let (stop, stopped) = oneshot::channel();
    let server = tokio::spawn(bound.serve_shutdown(app(), async {
        let _ = stopped.await;
    }));
    let mut client = BufReader::new(UnixStream::connect(&fixture.socket).await.unwrap());
    assert_eq!(
        exchange(&mut client, "CF-Connecting-IP: 192.0.2.17\r\n").await,
        (200, "192.0.2.17".into())
    );
    assert_eq!(
        exchange(&mut client, "CF-Connecting-IP: 2001:db8::17\r\n").await,
        (200, "2001:db8::17".into())
    );
    for headers in [
        "",
        "X-Forwarded-For: 192.0.2.17\r\n",
        "CF-Connecting-IP: 192.0.2.17\r\nCF-Connecting-IP: 192.0.2.17\r\n",
        "CF-Connecting-IP: 192.0.2.17,192.0.2.18\r\n",
        "CF-Connecting-IP: [2001:db8::17]:443\r\n",
    ] {
        assert_eq!(exchange(&mut client, headers).await.0, 403);
    }
    drop(client);
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!fixture.socket.exists());
    assert!(fixture.parent.exists());
    assert!(fixture.root.exists());
}

#[tokio::test]
async fn real_tcp_connection_does_not_trust_forwarded_ip_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app().into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .await
        .unwrap();
    });
    let mut client = BufReader::new(TcpStream::connect(address).await.unwrap());
    assert_eq!(
        exchange(
            &mut client,
            "CF-Connecting-IP: 192.0.2.17\r\nX-Forwarded-For: 192.0.2.17\r\n"
        )
        .await
        .0,
        403
    );
    drop(client);
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn cancelling_the_server_future_cleans_only_its_socket() {
    let fixture = support::Fixture::new();
    let bound = PreparedUnixIngress::prepare(&fixture.socket, &[])
        .unwrap()
        .bind()
        .unwrap();
    let server = tokio::spawn(bound.serve(app()));
    let mut client = BufReader::new(UnixStream::connect(&fixture.socket).await.unwrap());
    assert_eq!(
        exchange(&mut client, "CF-Connecting-IP: 192.0.2.17\r\n")
            .await
            .0,
        200
    );
    drop(client);
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
    assert!(!fixture.socket.exists());
}
