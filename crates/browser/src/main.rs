//! Top-level browser binary.
//!
//! **Primary owners**: Browser UX Team (process wiring is cross-team).
//!
//! This binary will eventually spawn Browser, Renderer, Network, GPU,
//! Utility/Storage, and DevTools processes per the architecture process model.
//! Teams own the crates; this crate only composes them.

use std::sync::Arc;

use js::Realm;
use webapis::{ConsoleSink, register_print};

struct StdoutConsole;

impl ConsoleSink for StdoutConsole {
    fn log(&self, message: &str) {
        println!("{message}");
    }
}

fn main() {
    let mut realm = Realm::new();
    register_print(&mut realm, Arc::new(StdoutConsole))
        .expect("registering built-in Web APIs should succeed");

    realm
        .evaluate_script("print()")
        .expect("the built-in print function should be callable");
}
