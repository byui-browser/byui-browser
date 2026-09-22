//! Bindings between JavaScript and DOM/platform APIs.
//!
//! **Owning team**: JS APIs (Web APIs) Team
//!
//! Window, Document, Element, Fetch, timers, events, and related APIs.
//! DOM mutations from JS must go through a controlled mutation interface
//! so style/layout invalidation stays correct.

#![forbid(unsafe_code)]

/// JavaScript-facing `print` function.
///
/// This is the first Web API binding exposed by this crate. The JavaScript
/// engine can register this function under the global `print` name and route
/// its output through the browser's console/devtools implementation once that
/// infrastructure is available.
pub fn print() {
    println!("JS API Team Rocks!");
}

#[cfg(test)]
mod tests {
    use super::print;

    #[test]
    fn print_binding_is_callable() {
        print();
    }
}
