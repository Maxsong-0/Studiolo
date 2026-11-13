//! 黑板客户端给失败定性（BUG-037）：权威服务确定拒绝（4xx）与这一刻拿不到结论
//! （连不上、5xx、正文读不完或不是 JSON）分开报。假黑板是只应答一次的手写 HTTP 服务。
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    thread::JoinHandle,
};
use studiolo_agent_server::board_client::{BoardCallError, BoardClient};
use tempfile::TempDir;

/// 假黑板对唯一一次连接的处理。
enum Reply {
    /// 读完请求后回这份完整报文。
    Raw(String),
    /// 接受连接后不读不写，立即关闭。
    Hangup,
}

/// 一份带 `content-length` 的完整 HTTP/1.1 应答。
fn http(status: &str, body: &str) -> Reply {
    let length = body.len();
    Reply::Raw(format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\n\
         content-length: {length}\r\nconnection: close\r\n\r\n{body}"
    ))
}

/// 读到请求头结束且正文按 `content-length` 读满为止；先读完再答，避免对端收到连接重置。
fn read_request(stream: &mut UnixStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buffer[..end]).to_lowercase();
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .map_or(0, |value| value.trim().parse::<usize>().unwrap());
            if buffer.len() >= end + 4 + length {
                return head;
            }
        }
        let read = stream.read(&mut chunk).unwrap();
        assert_ne!(read, 0, "请求没发完连接就关了");
        buffer.extend_from_slice(&chunk[..read]);
    }
}

/// 在临时目录里起一个只接受一次连接的假黑板。
fn serve_once(reply: Reply) -> (TempDir, PathBuf, JoinHandle<()>) {
    // 路径须短于 104 字节：放在 /tmp 下，不跟随可能很长的 TMPDIR。
    let temp = TempDir::new_in("/tmp").unwrap();
    let socket = temp.path().join("board.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let response = match reply {
            Reply::Raw(response) => response,
            Reply::Hangup => return,
        };
        let head = read_request(&mut stream);
        assert!(head.starts_with("post /internal/board "), "{head}");
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    });
    (temp, socket, server)
}

async fn call(socket: &Path) -> Result<Value, BoardCallError> {
    let client = BoardClient::new(socket).unwrap();
    client
        .call(&json!({"agent_type": "claude", "operation": "list", "request": {}}))
        .await
}

/// 起假黑板、调一次、等假黑板收尾，返回客户端的结论。
async fn call_once(reply: Reply) -> Result<Value, BoardCallError> {
    let (_temp, socket, server) = serve_once(reply);
    let result = call(&socket).await;
    server.join().unwrap();
    result
}

#[tokio::test]
async fn success_with_json_body_is_returned() {
    let result = call_once(http("200 OK", r#"{"items":[]}"#)).await;
    assert_eq!(result, Ok(json!({"items": []})));
}

#[tokio::test]
async fn client_error_is_a_rejection_with_the_authority_reason() {
    let result = call_once(http("409 Conflict", r#"{"error":"拒绝原因"}"#)).await;
    assert_eq!(result, Err(BoardCallError::Rejected("拒绝原因".into())));
}

#[tokio::test]
async fn client_error_without_json_is_still_a_rejection() {
    let result = call_once(http("400 Bad Request", "not json")).await;
    assert_eq!(
        result,
        Err(BoardCallError::Rejected("黑板请求被权威服务拒绝".into()))
    );
}

#[tokio::test]
async fn server_error_is_unavailable_not_a_rejection() {
    let body = r#"{"error":"黑板暂不可用，请重试。"}"#;
    let result = call_once(http("503 Service Unavailable", body)).await;
    assert!(
        matches!(result, Err(BoardCallError::Unavailable(_))),
        "{result:?}"
    );
}

#[tokio::test]
async fn server_error_without_json_is_unavailable() {
    let result = call_once(http("503 Service Unavailable", "<html>busy</html>")).await;
    assert!(
        matches!(result, Err(BoardCallError::Unavailable(_))),
        "{result:?}"
    );
}

#[tokio::test]
async fn success_without_json_is_unavailable() {
    let result = call_once(http("200 OK", "not json")).await;
    assert!(
        matches!(result, Err(BoardCallError::Unavailable(_))),
        "{result:?}"
    );
}

#[tokio::test]
async fn connection_closed_before_reply_is_unavailable() {
    let result = call_once(Reply::Hangup).await;
    assert!(
        matches!(result, Err(BoardCallError::Unavailable(_))),
        "{result:?}"
    );
}

#[tokio::test]
async fn missing_socket_is_unavailable() {
    let temp = TempDir::new_in("/tmp").unwrap();
    let result = call(&temp.path().join("absent.sock")).await;
    assert!(
        matches!(result, Err(BoardCallError::Unavailable(_))),
        "{result:?}"
    );
}
