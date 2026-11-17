//! JavaScript `fetch` binding.

use std::sync::{Arc, Mutex};

use js::{Function, HostFunction, JsError, JsResult, Object, Realm, Value};
use net::{
    CacheMode, CredentialsMode, FetchEnvironment, RedirectMode, Request, RequestController,
    RequestMode, ResponseType,
};

/// JavaScript-facing `fetch` implementation.
///
/// The networking crate performs Fetch policy and transport work. The JS
/// engine currently has no native Promise, stream, or byte-sequence values, so
/// this binding exposes an immediately-settled thenable and a Response-shaped
/// object whose `text()` and `json()` methods consume a buffered body.
pub fn fetch(
    controller: &RequestController,
    base_url: Option<&str>,
    arguments: &[Value],
) -> JsResult<Value> {
    let Some(input) = arguments.first() else {
        return rejected("TypeError: fetch() requires a URL");
    };
    let url = match input {
        Value::String(url) => url.clone(),
        Value::Object(request) => match request.get("url") {
            Some(Value::String(url)) => url,
            _ => return rejected("TypeError: fetch() input must be a URL string or Request"),
        },
        _ => return rejected("TypeError: fetch() input must be a URL string or Request"),
    };

    let mut request = Request::get(url);
    if let Some(base_url) = base_url {
        let environment = FetchEnvironment::from_url(
            net::Url::parse(base_url)
                .map_err(|error| JsError::runtime(format!("invalid document URL: {error}")))?,
        )
        .map_err(|error| JsError::runtime(error.to_string()))?;
        request.set_environment(environment);
    }
    request.set_initiator(net::InitiatorType::Fetch);
    apply_init(&mut request, arguments.get(1))?;

    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| JsError::new(format!("unable to start network runtime: {error}")))?;
    let response = match runtime.block_on(controller.fetch(request)) {
        Ok(response) => response,
        Err(error) => return rejected(error.to_string()),
    };
    let net::Response {
        response_type,
        status,
        status_text,
        headers,
        url,
        redirected,
        body: response_body,
        ..
    } = response;
    let body = runtime
        .block_on(response_body.bytes())
        .map_err(|error| JsError::new(error.to_string()))?;
    Ok(fulfilled(Value::Object(response_object(
        response_type,
        status,
        status_text,
        headers,
        url,
        redirected,
        body,
    ))))
}

fn apply_init(request: &mut Request, init: Option<&Value>) -> JsResult<()> {
    let Some(Value::Object(init)) = init else {
        if init.is_some() {
            return Err(JsError::new("TypeError: fetch() init must be an object"));
        }
        return Ok(());
    };
    if let Some(value) = init.get("method") {
        let Value::String(method) = value else {
            return Err(JsError::new("TypeError: Request method must be a string"));
        };
        request
            .set_method(method)
            .map_err(|error| JsError::new(error.to_string()))?;
    }
    if let Some(value) = init.get("headers") {
        let (Value::Object(headers) | Value::Array(headers)) = value else {
            return Err(JsError::new("TypeError: headers must be an object"));
        };
        for name in headers.keys() {
            let Some(value) = headers.get(&name) else {
                continue;
            };
            request
                .set_header(
                    net::HeaderName::try_from(name.as_str())
                        .map_err(|_| JsError::new("TypeError: invalid header name"))?,
                    net::HeaderValue::try_from(value.to_string())
                        .map_err(|_| JsError::new("TypeError: invalid header value"))?,
                )
                .map_err(|error| JsError::new(error.to_string()))?;
        }
    }
    if let Some(Value::String(body)) = init.get("body") {
        request
            .set_body(Some(net::Body::from_bytes(body.as_bytes().to_vec())))
            .map_err(|error| JsError::new(error.to_string()))?;
    }
    if let Some(Value::String(mode)) = init.get("mode") {
        request
            .set_mode(match mode.as_str() {
                "same-origin" => RequestMode::SameOrigin,
                "no-cors" => RequestMode::NoCors,
                "navigate" => RequestMode::Navigate,
                _ => RequestMode::Cors,
            })
            .map_err(|error| JsError::new(error.to_string()))?;
    }
    if let Some(Value::String(credentials)) = init.get("credentials") {
        request.set_credentials_mode(match credentials.as_str() {
            "omit" => CredentialsMode::Omit,
            "include" => CredentialsMode::Include,
            _ => CredentialsMode::SameOrigin,
        });
    }
    if let Some(Value::String(cache)) = init.get("cache") {
        request.set_cache_mode(match cache.as_str() {
            "no-store" => CacheMode::NoStore,
            "reload" => CacheMode::Reload,
            "no-cache" => CacheMode::NoCache,
            "force-cache" => CacheMode::ForceCache,
            "only-if-cached" => CacheMode::OnlyIfCached,
            _ => CacheMode::Default,
        });
    }
    if let Some(Value::String(redirect)) = init.get("redirect") {
        request.set_redirect_mode(match redirect.as_str() {
            "error" => RedirectMode::Error,
            "manual" => RedirectMode::Manual,
            _ => RedirectMode::Follow,
        });
    }
    Ok(())
}

fn response_object(
    response_type: ResponseType,
    status: u16,
    status_text: String,
    response_headers: net::Headers,
    url: String,
    redirected: bool,
    body: Vec<u8>,
) -> Object {
    let object = Object::new();
    object.set("status", Value::Number(status as f64));
    object.set("statusText", Value::String(status_text));
    object.set("ok", Value::Boolean((200..=299).contains(&status)));
    object.set("url", Value::String(url));
    object.set("redirected", Value::Boolean(redirected));
    object.set(
        "type",
        Value::String(response_type_name(response_type).into()),
    );

    let headers = Object::new();
    for (name, value) in response_headers.iter() {
        headers.set(name.as_str(), Value::String(value));
    }
    let headers_for_get = headers.clone();
    headers.set_method(
        "get",
        Arc::new(move |arguments| {
            let Some(Value::String(name)) = arguments.first() else {
                return Ok(Value::Null);
            };
            Ok(headers_for_get
                .get(&name.to_ascii_lowercase())
                .unwrap_or(Value::Null))
        }),
    );
    object.set("headers", Value::Object(headers));

    let body = Arc::new(Mutex::new(Some(body)));
    let text_body = Arc::clone(&body);
    object.set_method(
        "text",
        Arc::new(move |_| {
            let bytes = text_body.lock().unwrap().take().unwrap_or_default();
            String::from_utf8(bytes)
                .map(|body| fulfilled(Value::String(body)))
                .map_err(|_| JsError::new("TypeError: response body is not valid UTF-8"))
        }),
    );
    let json_body = Arc::clone(&body);
    object.set_method(
        "json",
        Arc::new(move |_| {
            let bytes = json_body.lock().unwrap().take().unwrap_or_default();
            let text =
                String::from_utf8(bytes).map_err(|_| JsError::new("SyntaxError: invalid JSON"))?;
            let realm = Realm::new();
            realm.evaluate_script(&format!("({text})")).map(fulfilled)
        }),
    );
    object
}

fn response_type_name(response_type: ResponseType) -> &'static str {
    match response_type {
        ResponseType::Basic => "basic",
        ResponseType::Cors => "cors",
        ResponseType::Opaque => "opaque",
        ResponseType::OpaqueRedirect => "opaqueredirect",
        ResponseType::Error => "error",
    }
}

fn fulfilled(value: Value) -> Value {
    settled_thenable(Some(value), None)
}

fn rejected(message: impl Into<String>) -> JsResult<Value> {
    Ok(settled_thenable(None, Some(message.into())))
}

fn settled_thenable(value: Option<Value>, error: Option<String>) -> Value {
    let promise = Object::new();
    let then_value = value.clone();
    promise.set_method(
        "then",
        Arc::new(move |arguments| {
            if let (Some(Value::Function(callback)), Some(value)) =
                (arguments.first(), then_value.clone())
            {
                return callback.call(&[value]);
            }
            Ok(Value::Undefined)
        }),
    );
    promise.set_method(
        "catch",
        Arc::new(move |arguments| {
            if let (Some(Value::Function(callback)), Some(error)) =
                (arguments.first(), error.clone())
            {
                return callback.call(&[Value::String(error)]);
            }
            Ok(Value::Undefined)
        }),
    );
    Value::Object(promise)
}

/// Installs the Web API global `fetch` in a JavaScript realm.
pub fn register_fetch(realm: &mut Realm, controller: Arc<RequestController>) -> JsResult<()> {
    register_fetch_with_url(realm, controller, None)
}

/// Installs `fetch` with the document URL used to resolve relative inputs.
pub fn register_fetch_with_url(
    realm: &mut Realm,
    controller: Arc<RequestController>,
    base_url: Option<&str>,
) -> JsResult<()> {
    let base_url = base_url.map(str::to_owned);
    let function: HostFunction =
        Arc::new(move |arguments| fetch(controller.as_ref(), base_url.as_deref(), arguments));
    let window_function = Function::from_host("fetch", Arc::clone(&function));
    realm.register_global_function("fetch", function)?;
    if let Some(Value::Object(window)) = realm.registered_global_value("window") {
        window.set("fetch", Value::Function(window_function));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use net::{Config, RequestController};

    #[test]
    fn invalid_input_returns_a_rejected_thenable() {
        let controller = RequestController::builder(Config::default())
            .build()
            .unwrap();
        let Value::Object(promise) = fetch(&controller, None, &[Value::Boolean(true)]).unwrap()
        else {
            panic!("fetch should return a promise-like object");
        };
        assert!(matches!(promise.get("then"), Some(Value::Function(_))));
        assert!(matches!(promise.get("catch"), Some(Value::Function(_))));
    }

    #[test]
    fn request_init_applies_method_headers_and_body() {
        let mut request = Request::get("https://example.test/");
        let init = Object::new();
        init.set("method", Value::String("POST".into()));
        let headers = Object::new();
        headers.set("x-test", Value::String("yes".into()));
        init.set("headers", Value::Object(headers));
        init.set("body", Value::String("payload".into()));

        apply_init(&mut request, Some(&Value::Object(init))).unwrap();

        assert_eq!(request.method().as_str(), "POST");
        assert_eq!(request.headers().get("x-test").as_deref(), Some("yes"));
        assert_eq!(request.body().unwrap().length(), Some(7));
    }

    #[test]
    fn registration_exposes_fetch_on_the_global_and_window() {
        let controller = Arc::new(
            RequestController::builder(Config::default())
                .build()
                .unwrap(),
        );
        let mut realm = Realm::new();
        crate::register_window(&mut realm).unwrap();
        register_fetch(&mut realm, controller).unwrap();

        assert!(realm.call_global("fetch", &[Value::Boolean(true)]).is_ok());
        let Some(Value::Object(window)) = realm.registered_global_value("window") else {
            panic!("window should be registered");
        };
        assert!(matches!(window.get("fetch"), Some(Value::Function(_))));
    }
}
