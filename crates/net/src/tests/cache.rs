//! Integration tests for response caching.

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{atomic::{AtomicUsize, Ordering}, Arc},
    thread,
};

use crate::{Request, RequestController};
use reqwest::StatusCode;

#[test]
fn cache_serves_a_fresh_get_without_a_second_network_request() {
    // The local server returns an explicit freshness lifetime. The second GET
    // should therefore be served from the controller's cache.
    let (address, request_count, server) = start_cached_server();
    let runtime = tokio::runtime::Runtime::new().expect("Tokio runtime should initialize");
    let controller = RequestController::new().expect("reqwest client should initialize");

    let (first, second) = runtime.block_on(async {
        let first = controller.execute(Request::get(&address)).await.expect("first request should succeed");
        let second = controller.execute(Request::get(&address)).await.expect("cached request should succeed");
        (first, second)
    });
    server.join().expect("test server should exit");

    assert_eq!(first.status, StatusCode::OK);
    assert!(!first.from_cache);
    assert_eq!(first.body, b"cached response");
    assert!(second.from_cache);
    assert_eq!(second.body, b"cached response");
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
}

fn start_cached_server() -> (String, Arc<AtomicUsize>, thread::JoinHandle<()>) {
    // Bind an ephemeral local port so this test does not depend on external
    // services or a fixed port that could already be in use.
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let address = format!("http://{}", listener.local_addr().expect("server address should exist"));
    let request_count = Arc::new(AtomicUsize::new(0));
    let count_for_server = Arc::clone(&request_count);
    let server = thread::spawn(move || {
        // This server intentionally accepts exactly one request. If the second
        // controller call reaches the network, the test will fail or hang.
        let (mut stream, _) = listener.accept().expect("server should receive one request");
        read_request(&mut stream);
        count_for_server.fetch_add(1, Ordering::SeqCst);
        let response = "HTTP/1.1 200 OK\r\nContent-Length: 15\r\nCache-Control: max-age=60\r\nConnection: close\r\n\r\ncached response";
        stream.write_all(response.as_bytes()).expect("server should write response");
    });

    (address, request_count, server)
}

fn read_request(stream: &mut TcpStream) {
    // Reading the request headers ensures the server does not race the client
    // before writing its response. The contents are not relevant to this test.
    let mut buffer = [0; 4096];
    let _ = stream.read(&mut buffer).expect("server should read request");
}
