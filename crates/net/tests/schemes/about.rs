//! Tests for the currently unsupported `about:` boundary.

use net::{Request, RequestError};

use super::support::{controller_with_services_and_providers, runtime};

#[test]
fn about_urls_are_rejected_without_calling_scheme_providers() {
    let (controller, _) = controller_with_services_and_providers();
    for url in ["about:blank", "about:srcdoc", "about:unknown"] {
        let error = runtime()
            .block_on(controller.fetch(Request::get(url)))
            .unwrap_err();
        assert!(matches!(
            error,
            RequestError::UnsupportedFeature("about scheme handler")
        ));
    }
}
