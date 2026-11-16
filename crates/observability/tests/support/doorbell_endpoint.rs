//! 门铃发送器对本机回环上的真实 HTTP 假服务：协议、状态码分类、不跟重定向、时限。
//! 不读写进程环境；密钥是公开文档里的示例值。

use super::*;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const SECRET: &str = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";

#[derive(Clone)]
enum Reply {
    Status(u16),
    Redirect(String),
    Stall,
}

struct Request {
    head: String,
    body: Vec<u8>,
}

struct Server {
    url: String,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new(reply: Reply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定假服务");
        let url = format!("http://{}/hook", listener.local_addr().expect("监听地址"));
        listener.set_nonblocking(true).expect("非阻塞监听");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (records, stopping) = (requests.clone(), stop.clone());
        let join = thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).expect("阻塞读取");
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .expect("读取预算");
                        records.lock().expect("请求记录").push(read(&mut stream));
                        match &reply {
                            Reply::Status(status) => {
                                let _ = write!(
                                    stream,
                                    "HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                );
                            }
                            Reply::Redirect(target) => {
                                let _ = write!(
                                    stream,
                                    "HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                );
                            }
                            Reply::Stall => {
                                while !stopping.load(Ordering::SeqCst) {
                                    thread::sleep(Duration::from_millis(2));
                                }
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("假服务 accept 失败：{error}"),
                }
            }
        });
        Self {
            url,
            requests,
            stop,
            join: Some(join),
        }
    }

    fn received(&self) -> usize {
        self.requests.lock().expect("请求记录").len()
    }

    fn only(&self) -> Request {
        let mut requests = self.requests.lock().expect("请求记录");
        assert_eq!(requests.len(), 1, "应当恰好收到一条请求");
        requests.remove(0)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            join.join().expect("假服务线程");
        }
    }
}

fn read(stream: &mut TcpStream) -> Request {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    let split = loop {
        if let Some(at) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break bytes.len(),
            Ok(count) => bytes.extend_from_slice(&chunk[..count]),
        }
    };
    let head = String::from_utf8_lossy(&bytes[..split]).into_owned();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    let mut body = bytes[split..].to_vec();
    while body.len() < length {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(count) => body.extend_from_slice(&chunk[..count]),
        }
    }
    Request { head, body }
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

fn endpoint(server: &Server) -> HttpEndpoint {
    HttpEndpoint::for_test(&server.url, SECRET, None, Duration::from_secs(5))
}

fn configured(values: &[(&str, &str)]) -> HttpEndpoint {
    HttpEndpoint::from_lookup(|name| {
        Ok(values
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| (*value).to_owned()))
    })
}

#[test]
fn a_signed_post_carries_the_three_headers_and_the_exact_body() {
    let server = Server::new(Reply::Status(204));
    let body = br#"{"data":{},"timestamp":"2026-11-15T00:00:00Z","type":"task.created"}"#;
    assert_eq!(
        endpoint(&server).post("msg_abc", 1_700_000_000, body),
        Ok(())
    );
    let request = server.only();
    assert!(request.head.starts_with("POST /hook HTTP/1.1"));
    assert_eq!(
        header(&request.head, "content-type"),
        Some("application/json")
    );
    assert_eq!(header(&request.head, "webhook-id"), Some("msg_abc"));
    assert_eq!(
        header(&request.head, "webhook-timestamp"),
        Some("1700000000")
    );
    assert_eq!(
        header(&request.head, "user-agent"),
        Some("studiolo-doorbell/1")
    );
    let expected = signature(
        &Secret::parse(SECRET).unwrap(),
        "msg_abc",
        1_700_000_000,
        body,
    )
    .unwrap();
    assert_eq!(
        header(&request.head, "webhook-signature"),
        Some(expected.as_str())
    );
    assert_eq!(request.body, body);
}

#[test]
fn status_codes_map_to_fixed_classifications() {
    for (status, expected) in [
        (200, Ok(())),
        (202, Ok(())),
        (410, Err(PostError::Gone)),
        (408, Err(PostError::Unavailable)),
        (429, Err(PostError::Unavailable)),
        (500, Err(PostError::Unavailable)),
        (503, Err(PostError::Unavailable)),
        (400, Err(PostError::Rejected)),
        (401, Err(PostError::Rejected)),
        (404, Err(PostError::Rejected)),
    ] {
        let server = Server::new(Reply::Status(status));
        assert_eq!(
            endpoint(&server).post("msg_abc", 1, b"{}"),
            expected,
            "状态码 {status}"
        );
        assert_eq!(server.received(), 1);
    }
}

#[test]
fn redirects_are_refused_and_never_followed() {
    let target = Server::new(Reply::Status(200));
    let server = Server::new(Reply::Redirect(target.url.clone()));
    assert_eq!(
        endpoint(&server).post("msg_abc", 1, b"{}"),
        Err(PostError::Rejected)
    );
    assert_eq!(server.received(), 1);
    assert_eq!(target.received(), 0, "不许把签过名的请求带到别的地址");
}

#[test]
fn a_stalled_receiver_times_out_within_the_budget() {
    let server = Server::new(Reply::Stall);
    let endpoint = HttpEndpoint::for_test(&server.url, SECRET, None, Duration::from_millis(200));
    let started = Instant::now();
    assert_eq!(endpoint.post("msg_abc", 1, b"{}"), Err(PostError::Timeout));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn a_closed_port_is_unavailable() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("占一个端口");
    let url = format!("http://{}/hook", listener.local_addr().expect("监听地址"));
    drop(listener);
    let endpoint = HttpEndpoint::for_test(&url, SECRET, None, Duration::from_secs(2));
    assert_eq!(
        endpoint.post("msg_abc", 1, b"{}"),
        Err(PostError::Unavailable)
    );
}

#[test]
fn malformed_message_ids_never_reach_the_network() {
    let server = Server::new(Reply::Status(200));
    let long = "m".repeat(65);
    for id in ["", "msg abc", "msg\r\nx-injected: 1", long.as_str()] {
        assert_eq!(
            endpoint(&server).post(id, 1, b"{}"),
            Err(PostError::Malformed)
        );
    }
    assert_eq!(server.received(), 0);
}

#[test]
fn configuration_is_all_or_nothing_and_fails_closed() {
    let url = (URL_VARIABLE, "https://hooks.example.test/in?route=a");
    let secret = (SECRET_VARIABLE, SECRET);
    let none = configured(&[]);
    assert!(!none.configured());
    assert_eq!(none.endpoint_tag(), Err(PostError::NotConfigured));
    assert_eq!(
        none.post("msg_abc", 1, b"{}"),
        Err(PostError::NotConfigured)
    );
    // 只给了链接也算没配：没有地址和密钥就没有门铃。
    let link_only = configured(&[(LINK_VARIABLE, "https://studiolo.example.test/")]);
    assert_eq!(link_only.endpoint_tag(), Err(PostError::NotConfigured));

    for broken in [
        vec![url],
        vec![secret],
        vec![(URL_VARIABLE, "http://hooks.example.test/in"), secret],
        vec![
            (URL_VARIABLE, "https://user:pass@hooks.example.test/in"),
            secret,
        ],
        vec![(URL_VARIABLE, "https://hooks.example.test/in#part"), secret],
        vec![url, (SECRET_VARIABLE, "not-a-webhook-secret")],
        vec![
            url,
            secret,
            (LINK_VARIABLE, "https://studiolo.example.test/?next=a"),
        ],
        vec![
            url,
            secret,
            (LINK_VARIABLE, "http://studiolo.example.test/"),
        ],
    ] {
        let endpoint = configured(&broken);
        assert!(!endpoint.configured());
        assert_eq!(endpoint.endpoint_tag(), Err(PostError::Malformed));
        assert_eq!(
            endpoint.post("msg_abc", 1, b"{}"),
            Err(PostError::Malformed)
        );
    }

    let full = configured(&[
        url,
        secret,
        (LINK_VARIABLE, "https://studiolo.example.test/"),
    ]);
    assert!(full.configured());
    assert_eq!(full.link(), Some("https://studiolo.example.test/"));
    let tag = full.endpoint_tag().unwrap();
    assert!(tag.len() == 16 && tag.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(configured(&[url, secret]).endpoint_tag().unwrap(), tag);
    let moved = configured(&[(URL_VARIABLE, "https://hooks.example.test/other"), secret]);
    assert_ne!(moved.endpoint_tag().unwrap(), tag);
    // Debug 不带地址和密钥。
    let shown = format!("{full:?}");
    assert!(!shown.contains("hooks.example") && !shown.contains("whsec_"));
}
