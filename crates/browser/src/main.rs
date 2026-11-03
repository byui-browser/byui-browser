//! Top-level browser binary: evaluates the browser's built-in script demo and
//! opens the sample HTML page in a native webview.

use std::sync::Arc;

use browser::evaluate_page_scripts;
use tao::{event_loop::{ControlFlow, EventLoop}, window::WindowBuilder};
use webapis::ConsoleSink;
use wry::WebViewBuilder;

const DEMO_HTML: &str = include_str!("../../../browser-pages/demo.html");

struct StdoutConsole;

impl ConsoleSink for StdoutConsole {
    fn log(&self, message: &str) {
        println!("{message}");
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Keep the original browser-side JavaScript and Web API smoke demo.
    let controller = Arc::new(net::RequestController::new(net::Config::default())?);
    evaluate_page_scripts(
        "<script>print()</script>",
        Arc::new(StdoutConsole),
        controller,
    )?;

    // 2. Create a native window and the sample HTML webview.
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Interactive Rust Browser")
        .build(&event_loop)?;
    let (tx, rx) = std::sync::mpsc::channel();
    let webview = WebViewBuilder::new()
        .with_html(DEMO_HTML)
        .with_ipc_handler(move |request| {
            let _ = tx.send(request.into_body());
        })
        .build(&window)?;

    // 3. Handle window events and button messages.
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(
            std::time::Instant::now() + std::time::Duration::from_millis(16),
        );

        if let Ok(message) = rx.try_recv() {
            if message == "go_to_google" {
                let _ = webview.load_url("https://google.com");
            }
        }

        if let tao::event::Event::WindowEvent {
            event: tao::event::WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    });
}
