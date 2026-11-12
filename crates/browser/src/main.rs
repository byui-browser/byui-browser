//! Top-level browser binary: evaluates the browser's built-in script demo and
//! opens the welcome page in a native webview.

use std::sync::Arc;

use browser::{BrowserRuntime, application_data_directory, request_controller};
use rfd::{MessageButtons, MessageDialog, MessageLevel};
use security::{Origin, RendererSecurityContext, StoragePermission};
use tao::{
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use webapis::{AlertSink, ConsoleSink};
use wry::WebViewBuilder;

struct StdoutConsole;

impl ConsoleSink for StdoutConsole {
    fn log(&self, message: &str) {
        println!("{message}");
    }
}

/// Displays browser JavaScript alerts with the native platform dialog.
struct NativeAlert;

impl AlertSink for NativeAlert {
    fn alert(&self, message: &str) {
        MessageDialog::new()
            .set_title("BYUI Browser")
            .set_description(message)
            .set_level(MessageLevel::Info)
            .set_buttons(MessageButtons::Ok)
            .show();
    }
}

fn main() {
    let default_log = browser::default_log_file().ok();
    let command_line =
        common::logging::init_from_command_line_with_default_file(default_log.clone());
    if command_line.help_requested {
        println!(
            "Usage: browser [options]\n\n{}\n\n{}",
            common::logging::USAGE,
            browser::log_file_help(default_log.as_deref())
        );
        return;
    }
    for argument in &command_line.remaining {
        log::warn!("ignoring unrecognized argument `{argument}`");
    }
    log::info!("starting BYUI Browser {}", env!("CARGO_PKG_VERSION"));

    // 1. Initialize the browser runtime and keep its script smoke demo.
    let controller = request_controller(None);
    let runtime = BrowserRuntime::open(
        application_data_directory().expect("application data directory should be available"),
    )
    .expect("browser storage should initialize");
    let security_context = RendererSecurityContext::new(
        Origin::new("byui", "welcome", None),
        StoragePermission::Allowed,
    );
    runtime
        .evaluate_page_scripts(
            "<script>print()</script>",
            Arc::new(StdoutConsole),
            Arc::new(NativeAlert),
            controller,
            security_context,
        )
        .expect("page scripts should evaluate");

    // 2. Build the welcome page for the native webview, embedding its CSS.
    let page = browser::welcome_page();
    let button_markup = "<button id=\"btn\" class=\"google-button\" onclick=\"window.ipc.postMessage('go_to_google')\">Click to go to Google</button>";
    let html = page
        .html
        .replace(
            "<link rel=\"stylesheet\" href=\"style.css\">",
            &format!("<style>{}</style>", page.stylesheets.join("\n")),
        )
        .replace("<button id=\"btn\">Click Me</button>", button_markup)
        .replace("<script src=\"script.js\"></script>", "")
        .replace(
            "</head>",
            "<style>.google-button { width: auto; height: auto; padding: 12px 24px; border: 0; border-radius: 6px; background: #6f42c1; color: white; cursor: pointer; } .google-button:hover { background: #59339d; }</style></head>",
        );

    // 3. Open the window and route the page button to Google.
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Interactive Rust Browser")
        .build(&event_loop)
        .expect("browser window should open");
    let (tx, rx) = std::sync::mpsc::channel();
    let webview = WebViewBuilder::new()
        .with_html(&html)
        .with_ipc_handler(move |request| {
            let _ = tx.send(request.into_body());
        })
        .build(&window)
        .expect("browser webview should build");

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(
            std::time::Instant::now() + std::time::Duration::from_millis(16),
        );

        if let Ok(message) = rx.try_recv()
            && message == "go_to_google"
        {
            let _ = webview.load_url("https://google.com");
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
