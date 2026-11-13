//! 全部请求只到持有监听句柄的 loopback 假服务，不读写进程环境。

use super::super::*;
use crate::alert::{AlertSeverity, IdempotencyKey};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

#[derive(Clone)]
pub(super) enum Reply {
    Json(u16, String),
    Redirect(String),
    Disconnect,
    Stall,
    BodyStall,
    Slow,
}

impl Reply {
    pub(super) fn json(status: u16, body: &str) -> Self {
        Self::Json(status, body.into())
    }
}

#[derive(Clone)]
pub(super) struct Request {
    pub(super) head: String,
    pub(super) body: String,
}

pub(super) struct Server {
    pub(super) url: String,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    pub(super) peak: Arc<AtomicUsize>,
    join: Option<thread::JoinHandle<()>>,
}

impl Server {
    pub(super) fn new(reply: Reply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定假服务");
        let url = format!("http://{}/push", listener.local_addr().expect("监听地址"));
        listener.set_nonblocking(true).expect("非阻塞监听");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicUsize::new(0));
        let (records, stopping, maximum) = (requests.clone(), stop.clone(), peak.clone());
        let join = thread::spawn(move || {
            let mut workers = Vec::new();
            let active = Arc::new(AtomicUsize::new(0));
            while !stopping.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let records = records.clone();
                        let stopping = stopping.clone();
                        let maximum = maximum.clone();
                        let active = active.clone();
                        let reply = reply.clone();
                        workers.push(thread::spawn(move || {
                            stream
                                .set_read_timeout(Some(Duration::from_secs(2)))
                                .expect("读取预算");
                            records.lock().expect("请求记录").push(read_request(&mut stream));
                            maximum.fetch_max(
                                active.fetch_add(1, Ordering::SeqCst) + 1,
                                Ordering::SeqCst,
                            );
                            match reply {
                                Reply::Json(status, body) => respond(&mut stream, status, &body),
                                Reply::Redirect(url) => {
                                    let _ = write!(stream,
                                        "HTTP/1.1 302 Found\r\nLocation: {url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                    );
                                }
                                Reply::Disconnect => {}
                                Reply::Slow => {
                                    thread::sleep(Duration::from_millis(50));
                                    // 发送完整回执前结束活跃区间，避免服务器调度尾差误计并发。
                                    active.fetch_sub(1, Ordering::SeqCst);
                                    respond(&mut stream, 200, "{\"status\":1}");
                                    return;
                                }
                                reply @ (Reply::Stall | Reply::BodyStall) => {
                                    if matches!(reply, Reply::BodyStall) {
                                        let _ = stream.write_all(
                                            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n"
                                        );
                                    }
                                    while !stopping.load(Ordering::SeqCst) {
                                        thread::sleep(Duration::from_millis(2));
                                    }
                                }
                            }
                            active.fetch_sub(1, Ordering::SeqCst);
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("假服务 accept 失败：{error}"),
                }
            }
            for worker in workers {
                worker.join().expect("假服务请求线程");
            }
        });
        Self {
            url,
            requests,
            stop,
            peak,
            join: Some(join),
        }
    }

    pub(super) fn calls(&self) -> usize {
        self.requests.lock().expect("请求记录").len()
    }

    pub(super) fn request(&self) -> Request {
        self.requests.lock().expect("请求记录")[0].clone()
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

fn read_request(stream: &mut TcpStream) -> Request {
    let mut bytes = Vec::new();
    loop {
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let head = String::from_utf8(bytes[..end].to_vec()).expect("请求头");
            let size = head
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                .map(|(_, value)| value.trim().parse::<usize>().expect("请求大小"))
                .expect("有界请求体");
            if bytes.len() >= end + 4 + size {
                return Request {
                    head,
                    body: String::from_utf8(bytes[end + 4..end + 4 + size].to_vec())
                        .expect("请求正文"),
                };
            }
        }
        let mut chunk = [0; 1024];
        let count = stream.read(&mut chunk).expect("读取假请求");
        assert!(count > 0 && bytes.len() + count <= 16_384, "请求完整且有界");
        bytes.extend_from_slice(&chunk[..count]);
    }
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

pub(super) fn values() -> BTreeMap<&'static str, String> {
    [
        (TOKEN, "fixture-token"),
        (USER, "fixture-user"),
        (DEVICE, "fixture-device"),
        (LINK, "https://example.test/status"),
    ]
    .into_iter()
    .map(|(key, value)| (key, value.into()))
    .collect()
}

pub(super) fn transport(
    main: &Server,
    backup: &Server,
    values: BTreeMap<&str, String>,
) -> PushTransport {
    let mut transport = PushTransport::from_lookup(|name| Ok(values.get(name).cloned()));
    transport.http = http::Options::for_test(&main.url, &backup.url, Duration::from_secs(2));
    transport
}

pub(super) fn message(level: AlertSeverity, code: &'static str) -> AlertMessage {
    AlertMessage::new(
        IdempotencyKey::derive("push-fixture", &[code]),
        level,
        code,
        "不应外发的保密摘要",
    )
    .expect("合法测试消息")
}
