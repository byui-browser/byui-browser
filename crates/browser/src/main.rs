//! Top-level browser binary.
//!
//! **Primary owners**: Browser UX Team (process wiring is cross-team).
//!
//! This binary will eventually spawn Browser, Renderer, Network, GPU,
//! Utility/Storage, and DevTools processes per the architecture process model.
//! Teams own the crates; this crate only composes them.

use std::sync::Arc;

use browser::evaluate_page_scripts;
use webapis::{AlertSink, ConsoleSink};

struct StdoutConsole;

impl ConsoleSink for StdoutConsole {
    fn log(&self, message: &str) {
        println!("{message}");
    }
}

struct StdoutAlert;

impl AlertSink for StdoutAlert {
    fn alert(&self, message: &str) {
        println!("[alert] {message}");
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
    evaluate_page_scripts(
        "<script>print()</script>",
        Arc::new(StdoutConsole),
        Arc::new(StdoutAlert),
        controller,
    )
    .expect("page scripts should evaluate");
}
