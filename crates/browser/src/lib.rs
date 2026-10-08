//! Browser-side composition of HTML, JavaScript, and Web APIs.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::Arc;

use js::{JsResult, Realm, Value};
use net::RequestController;
use webapis::{ConsoleSink, register_fetch, register_print};

/// Parses `html_source`, installs the browser's Web APIs, and evaluates each
/// inline page script in document order.
///
/// The scripts are evaluated by one [`Realm`], so every script uses the same
/// registered host-function path. DOM mutation and scheduling remain outside
/// this entry point until those APIs are supported by the engine.
pub fn evaluate_page_scripts(
    html_source: &str,
    console: Arc<dyn ConsoleSink>,
    controller: Arc<RequestController>,
) -> JsResult<Vec<Value>> {
    let document = html::parse_raw_html(html_source.to_owned());
    let mut realm = Realm::new();
    register_print(&mut realm, console)?;
    register_fetch(&mut realm, controller)?;

    document
        .script_sources()
        .iter()
        .map(|source| realm.evaluate_script(source))
        .collect()
}
