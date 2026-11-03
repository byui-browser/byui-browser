//! Console-only refreshing through the shared networking API.

use std::io::{self, Write};

use chrome::BrowserState;
use net::{CacheMode, Config, Request, RequestController, SchemeProviders};
use tokio::{runtime::Runtime, task::JoinHandle};

/// Owns a reusable fetch controller and background runtime for Reload clicks.
///
/// Refresh prints responses without navigating, rendering fetched content, or
/// modifying history. Internal `byui:` pages use the existing embedded provider;
/// HTTP(S) requests bypass the controller's response cache. Dropping this client
/// shuts down its runtime, cancelling outstanding asynchronous requests.
pub struct RefreshClient {
    runtime: Runtime,
    controller: RequestController,
}

impl RefreshClient {
    /// Initializes the runtime and controller without fetching a page.
    ///
    /// Returns initialization failures so the shell can report them without
    /// panicking. Retain one client for the lifetime of the browser window.
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let runtime = Runtime::new()?;
        let controller = RequestController::builder(Config::default())
            .with_scheme_providers(
                SchemeProviders::new().with_page(crate::byui_provider::provider()),
            )
            .build()?;
        Ok(Self {
            runtime,
            controller,
        })
    }

    /// Copies the current URL and schedules a fresh fetch on a background task.
    ///
    /// Prints the requested URL, status and body to stdout, or a failure to
    /// stderr. Returns a task handle that may be dropped without cancelling the
    /// fetch. With no current URL, prints a diagnostic and returns `None`.
    /// The borrowed state is never mutated or retained by the task.
    pub fn refresh(&self, state: &BrowserState) -> Option<JoinHandle<()>> {
        let Some(url) = state.current_url().map(str::to_owned) else {
            let _ = writeln!(io::stderr().lock(), "Refresh skipped: no current URL.");
            return None;
        };
        let controller = self.controller.clone();
        Some(self.runtime.spawn(async move {
            match report(&controller, &url).await {
                Ok(message) => {
                    let _ = writeln!(io::stdout().lock(), "{message}");
                }
                Err(message) => {
                    let _ = writeln!(io::stderr().lock(), "{message}");
                }
            }
        }))
    }
}

async fn report(controller: &RequestController, url: &str) -> Result<String, String> {
    let mut request = Request::get(url);
    request.set_cache_mode(CacheMode::Reload);
    let response = controller
        .fetch(request)
        .await
        .map_err(|error| format!("Refresh failed for {url}: {error}"))?;
    let status = response.status;
    let body = response
        .body
        .text()
        .await
        .map_err(|error| format!("Refresh failed for {url} (status {status}): {error}"))?;
    Ok(format!("Refresh {url}\nStatus: {status}\n{body}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Read,
        net::TcpListener,
        thread,
        time::{Duration, Instant},
    };

    // Bounded local fixture: failures time out instead of hanging the test run.
    fn server(responses: Vec<String>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/saved", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let handle = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut requests = Vec::new();
            for response in responses {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "missing refresh request");
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("test server failed: {error}"),
                    }
                };
                // macOS inherits the listener's nonblocking mode on accept.
                // Request bytes may arrive later than the connection itself;
                // use bounded blocking reads rather than racing the client.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                requests.push(String::from_utf8(request).unwrap());
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, handle)
    }

    fn response(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nCache-Control: max-age=60\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    #[test]
    fn http_fixture_waits_for_delayed_request_headers() {
        let (url, server) = server(vec![response("200 OK", "fixture ready")]);
        let address = url
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap();
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        // Let accept run before sending headers, then split them across writes.
        thread::sleep(Duration::from_millis(50));
        stream
            .write_all(b"GET /saved HTTP/1.1\r\nHost: localhost\r\n")
            .unwrap();
        thread::sleep(Duration::from_millis(50));
        stream.write_all(b"\r\n").unwrap();
        let mut received = String::new();
        stream.read_to_string(&mut received).unwrap();
        assert_eq!(received, response("200 OK", "fixture ready"));
        assert_eq!(
            server.join().unwrap(),
            vec!["GET /saved HTTP/1.1\r\nHost: localhost\r\n\r\n"]
        );
    }

    #[test]
    fn refresh_returns_while_the_response_is_pending() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "missing refresh request");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("test server failed: {error}"),
                }
            };
            accepted_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            stream
                .write_all(response("200 OK", "ready").as_bytes())
                .unwrap();
        });
        let client = RefreshClient::new().unwrap();
        let mut state = BrowserState::new();
        state.record_visit(&url);
        let before = state.clone();
        let task = client.refresh(&state).unwrap();
        accepted_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(!task.is_finished());
        assert_eq!(state, before);
        release_tx.send(()).unwrap();
        client.runtime.block_on(task).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn refresh_bypasses_a_populated_http_cache() {
        let (url, server) = server(vec![response("200 OK", "old"), response("200 OK", "new")]);
        let client = RefreshClient::new().unwrap();
        client.runtime.block_on(async {
            let first = client.controller.fetch(Request::get(&url)).await.unwrap();
            assert_eq!(first.body.text().await.unwrap(), "old");
            let cached = client.controller.fetch(Request::get(&url)).await.unwrap();
            assert!(cached.from_cache);
            assert_eq!(cached.body.text().await.unwrap(), "old");
            let message = report(&client.controller, &url).await.unwrap();
            assert_eq!(message, format!("Refresh {url}\nStatus: 200\nnew"));
        });
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[test]
    fn refresh_uses_saved_url_snapshot_without_changing_history() {
        let (url, server) = server(vec![response("200 OK", "saved page")]);
        let client = RefreshClient::new().unwrap();
        let mut state = BrowserState::new();
        state.record_visit("byui://welcome");
        state.record_visit(&url);
        let before = state.clone();
        let task = client.refresh(&state).unwrap();
        assert_eq!(state, before);
        // A later visit cannot change the URL already submitted by Reload.
        state.record_visit("byui://welcome");
        client.runtime.block_on(task).unwrap();
        assert_eq!(
            state.visited_urls(),
            &["byui://welcome", &url, "byui://welcome"]
        );
        assert!(server.join().unwrap()[0].starts_with("GET /saved HTTP/1.1\r\n"));
    }

    #[test]
    fn internal_welcome_is_fetched_through_the_provider() {
        let client = RefreshClient::new().unwrap();
        let message = client
            .runtime
            .block_on(report(&client.controller, "byui://welcome"))
            .unwrap();
        assert!(message.starts_with("Refresh byui://welcome\nStatus: 200\n"));
        assert!(message.contains("href=\"style.css\""));
        assert!(message.contains("src=\"script.js\""));
    }

    #[test]
    fn missing_url_does_not_start_a_task_or_change_state() {
        let client = RefreshClient::new().unwrap();
        let state = BrowserState::new();
        assert!(client.refresh(&state).is_none());
        assert_eq!(state, BrowserState::new());
    }

    #[test]
    fn fetch_errors_are_reported_and_leave_history_unchanged() {
        let client = RefreshClient::new().unwrap();
        for url in ["not a URL", "byui://missing"] {
            let mut state = BrowserState::new();
            state.record_visit(url);
            let before = state.clone();
            let message = client
                .runtime
                .block_on(report(&client.controller, url))
                .unwrap_err();
            assert!(message.starts_with(&format!("Refresh failed for {url}:")));
            client
                .runtime
                .block_on(client.refresh(&state).unwrap())
                .unwrap();
            assert_eq!(state, before);
        }
    }

    #[test]
    fn http_error_status_and_body_are_printable_results() {
        let (url, server) = server(vec![response("404 Not Found", "missing page")]);
        let client = RefreshClient::new().unwrap();
        let message = client
            .runtime
            .block_on(report(&client.controller, &url))
            .unwrap();
        assert_eq!(message, format!("Refresh {url}\nStatus: 404\nmissing page"));
        server.join().unwrap();
    }

    #[test]
    fn incomplete_response_body_is_a_reported_failure() {
        let (url, server) = server(vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort".into(),
        ]);
        let client = RefreshClient::new().unwrap();
        let message = client
            .runtime
            .block_on(report(&client.controller, &url))
            .unwrap_err();
        assert!(message.starts_with(&format!("Refresh failed for {url} (status 200):")));
        server.join().unwrap();
    }
}
