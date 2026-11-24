#![allow(dead_code)]

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use net::{RequestError, Response};

pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("Tokio runtime should initialize")
}

pub async fn collect_body(response: Response) -> Result<Vec<u8>, RequestError> {
    response.body.bytes().await
}

pub struct TestServer {
    url: String,
    thread: thread::JoinHandle<()>,
    stop: Arc<AtomicBool>,
}

impl TestServer {
    /// Starts a minimal HTTP/1.1 server that remains available until `join`.
    /// `connections` is retained for compatibility with existing test call sites.
    pub fn start<F>(_connections: usize, response: F) -> Self
    where
        F: Fn(&str) -> Vec<u8> + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("server should bind");
        listener
            .set_nonblocking(true)
            .expect("server should set nonblocking mode");
        let url = format!("http://{}", listener.local_addr().unwrap());
        let response = Arc::new(response);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !worker_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_nonblocking(false)
                            .expect("server should set accepted socket to blocking");
                        let request = read_request(&mut stream);
                        stream
                            .write_all(response(&request).as_slice())
                            .expect("server should write response");
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("server should accept request: {error}"),
                }
            }
        });
        Self { url, thread, stop }
    }

    pub fn url(&self) -> String {
        self.url.clone()
    }

    pub fn join(self) {
        self.stop.store(true, Ordering::SeqCst);
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

#[test]
#[ignore = "For demonstration only."]
pub fn fetch_image() {
    // Fetches an image using the net crate and saves it to a file.
    let url = "https://picsum.photos/200/300";
    let runtime = runtime();
    let body = runtime.block_on(async {
        let controller = net::RequestController::builder(net::Config::default())
            .build()
            .expect("request controller should initialize");
        controller
            .fetch(net::Request::get(url))
            .await
            .expect("request should succeed")
            .body
            .bytes()
            .await
            .expect("response body should be readable")
    });
    std::fs::write("picture.png", body).expect("file should be written");
}
