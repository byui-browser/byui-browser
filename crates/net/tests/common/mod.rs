#![allow(dead_code)]

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    thread,
};

use futures_util::StreamExt;
use net::{RequestError, StreamingResponse};

pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("Tokio runtime should initialize")
}

pub async fn collect_body(mut response: StreamingResponse) -> Result<Vec<u8>, RequestError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.body.next().await {
        body.extend_from_slice(&chunk?);
    }
    Ok(body)
}

pub struct TestServer {
    url: String,
    thread: thread::JoinHandle<()>,
}

impl TestServer {
    /// Starts a minimal HTTP/1.1 server that accepts exactly `connections`
    /// requests, making each test's expected network activity explicit.
    pub fn start<F>(connections: usize, response: F) -> Self
    where
        F: Fn(&str) -> Vec<u8> + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("server should bind");
        let url = format!("http://{}", listener.local_addr().unwrap());
        let response = Arc::new(response);
        let thread = thread::spawn(move || {
            for _ in 0..connections {
                let (mut stream, _) = listener.accept().expect("server should accept request");
                let request = read_request(&mut stream);
                stream
                    .write_all(response(&request).as_slice())
                    .expect("server should write response");
            }
        });
        Self { url, thread }
    }

    pub fn url(&self) -> String {
        self.url.clone()
    }

    pub fn join(self) {
        self.thread.join().expect("server should exit");
    }
}

pub fn read_request(stream: &mut TcpStream) -> String {
    // Read through the request body so assertions inspect the complete request
    // even when the operating system splits it across multiple TCP reads.
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    loop {
        let count = stream
            .read(&mut buffer)
            .expect("server should read request");
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let body_length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':').and_then(|(name, value)| {
                        name.eq_ignore_ascii_case("content-length")
                            .then_some(value.trim())
                    })
                })
                .and_then(|length| length.parse::<usize>().ok())
                .unwrap_or(0);
            if bytes.len() >= header_end + 4 + body_length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
