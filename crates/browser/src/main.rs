//! Top-level browser binary.
//!
//! **Primary owners**: Browser UX Team (process wiring is cross-team).
//!
//! This binary will eventually spawn Browser, Renderer, Network, GPU,
//! Utility/Storage, and DevTools processes per the architecture process model.
//! Teams own the crates; this crate only composes them.

use std::sync::Arc;

use browser::evaluate_page_scripts;
use webapis::ConsoleSink;

struct StdoutConsole;

impl ConsoleSink for StdoutConsole {
    fn log(&self, message: &str) {
        println!("{message}");
    }
}

fn main() {
    // Keep the standalone binary useful while native platform shells are
    // brought online. `make run` selects the platform shell below.
    let controller = Arc::new(
        net::RequestController::new(net::Config::default())
            .expect("network client should initialize"),
    );
    evaluate_page_scripts(
        "<script>print()</script>",
        Arc::new(StdoutConsole),
        controller,
    )
    .expect("page scripts should evaluate");
}
