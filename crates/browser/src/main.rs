//! Top-level browser binary.
//!
//! **Primary owners**: Browser UX Team (process wiring is cross-team).
//!
//! This binary will eventually spawn Browser, Renderer, Network, GPU,
//! Utility/Storage, and DevTools processes per the architecture process model.
//! Teams own the crates; this crate only composes them.

use std::sync::Arc;

use browser::{BrowserRuntime, application_data_directory};
use rfd::{MessageButtons, MessageDialog, MessageLevel};
use security::{Origin, RendererSecurityContext, StoragePermission};
use webapis::{AlertSink, ConsoleSink};

struct StdoutConsole;

impl ConsoleSink for StdoutConsole {
    fn log(&self, message: &str) {
        println!("{message}");
    }
}

/// Native modal alert adapter for the standalone browser composition binary.
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
    // Keep the standalone binary useful while native platform shells are
    // brought online. `make run` selects the platform shell below.
    let providers = net::SchemeProviders::new().with_page(browser::byui_provider::provider());
    let controller = Arc::new(
        net::RequestController::builder(net::Config::default())
            .with_scheme_providers(providers)
            .build()
            .expect("network client should initialize"),
    );
    let runtime = BrowserRuntime::open(
        application_data_directory().expect("application data directory should be available"),
    )
    .expect("browser storage should initialize");
    // The built-in page uses a fixed origin until navigation supplies a URL.
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
}
