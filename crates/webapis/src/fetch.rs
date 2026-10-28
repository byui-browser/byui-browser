//! JavaScript `fetch` binding.

use std::sync::Arc;

use js::{HostFunction, JsError, JsResult, Realm, Value};
use net::{Request, RequestController};

/// JavaScript-facing `fetch` implementation.
///
/// The current JavaScript value model has no object or promise values, so this
/// first binding exposes the response body as a string. Network execution is
/// delegated to the networking crate's shared request controller.
pub fn fetch(controller: &RequestController, arguments: &[Value]) -> JsResult<Value> {
    let [Value::String(url)] = arguments else {
        return Err(JsError::new("fetch() expects exactly one URL string"));
    };

    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| JsError::new(format!("unable to start network runtime: {error}")))?;
    let response = runtime
        .block_on(controller.fetch(Request::get(url)))
        .map_err(|error| JsError::new(error.to_string()))?;

    let body = runtime
        .block_on(response.body.bytes())
        .map_err(|error| JsError::new(error.to_string()))?;
    let body = String::from_utf8(body)
        .map_err(|_| JsError::new("fetch() response body is not valid UTF-8"))?;
    Ok(Value::String(body))
}

/// Installs the Web API global `fetch` in a JavaScript realm.
pub fn register_fetch(realm: &mut Realm, controller: Arc<RequestController>) -> JsResult<()> {
    let function: HostFunction = Arc::new(move |arguments| fetch(controller.as_ref(), arguments));
    realm.register_global_function("fetch", function)
}

#[cfg(test)]
mod tests {
    use js::{Realm, Value};
    use net::{Config, RequestController};

    use super::*;

    #[test]
    fn fetch_requires_one_url_string() {
        let controller = RequestController::builder(Config::default())
            .build()
            .unwrap();

        assert_eq!(
            fetch(&controller, &[]).unwrap_err().to_string(),
            "fetch() expects exactly one URL string"
        );
        assert_eq!(
            fetch(&controller, &[Value::Boolean(true)])
                .unwrap_err()
                .to_string(),
            "fetch() expects exactly one URL string"
        );
    }

    #[test]
    fn registration_exposes_fetch_to_the_realm() {
        let controller = std::sync::Arc::new(
            RequestController::builder(Config::default())
                .build()
                .unwrap(),
        );
        let mut realm = Realm::new();
        register_fetch(&mut realm, controller).unwrap();

        assert_eq!(
            realm.evaluate_script("fetch()").unwrap_err().to_string(),
            "fetch() expects exactly one URL string"
        );
    }
}
