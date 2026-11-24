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

const SOCKET_TIMEOUT: Duration = Duration::from_secs(5);

pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("Tokio runtime should initialize")
}

pub async fn collect_body(response: Response) -> Result<Vec<u8>, RequestError> {
    response.body.bytes().await
}

pub struct TestServer {
    url: String,
    thread: Option<thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

impl TestServer {
    /// Starts a minimal concurrent HTTP/1.1 server that remains available until `join`.
    ///
    /// `expected_connections` is a sizing hint. Each accepted connection runs
    /// on its own worker so tests can exercise transport concurrency.
    pub fn start<F>(expected_connections: usize, response: F) -> Self
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
            let mut workers = Vec::with_capacity(expected_connections);
            while !worker_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_nonblocking(false)
                            .expect("server should set accepted socket to blocking");
                        stream
                            .set_read_timeout(Some(SOCKET_TIMEOUT))
                            .expect("server should set read timeout");
                        stream
                            .set_write_timeout(Some(SOCKET_TIMEOUT))
                            .expect("server should set write timeout");
                        let response = Arc::clone(&response);
                        workers.push(thread::spawn(move || {
                            let request = read_request(&mut stream);
                            stream
                                .write_all(response(&request).as_slice())
                                .expect("server should write response");
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("server should accept request: {error}"),
                }
            }
            for worker in workers {
                worker.join().expect("server connection worker should exit");
            }
        });
        Self {
            url,
            thread: Some(thread),
            stop,
        }
    }

    pub fn url(&self) -> String {
        self.url.clone()
    }

    pub fn join(mut self) {
        self.stop_and_join(true);
    }

    fn stop_and_join(&mut self, propagate_panic: bool) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let result = thread.join();
            if propagate_panic {
                result.expect("server should exit");
            }
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop_and_join(false);
    }
}

pub fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(SOCKET_TIMEOUT))
        .expect("server should set request read timeout");
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
