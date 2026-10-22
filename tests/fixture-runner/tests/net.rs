//! Net team fixtures: `input.http` (a recorded response, served from
//! 127.0.0.1) -> response dump. The URL is left out because the port changes.
use fixture_runner::{fetch, read_http, run_group, serve_once};

#[test]
fn net_fixtures() {
    run_group("net", |case| {
        let url = serve_once(read_http(case));
        match fetch(&url) {
            Ok((response, body)) => {
                let mut out = format!(
                    "status: {} {:?}\nredirected: {}\n",
                    response.status, response.status_text, response.redirected
                );
                for (name, value) in response.headers.iter() {
                    out.push_str(&format!("header: {name}: {value}\n"));
                }
                out.push_str(&format!(
                    "body: {} bytes\n{}\n",
                    body.len(),
                    String::from_utf8_lossy(&body)
                ));
                out
            }
            Err(error) => format!("Err: {error:?}\n"),
        }
    });
}
