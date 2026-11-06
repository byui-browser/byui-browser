//! The JavaScript `Location` interface and browser navigation requests.

use std::sync::{Arc, Mutex};

use js::{HostFunction, HostGetter, HostSetter, JsError, JsResult, Object, Realm, Value};
use net::Url;

/// History behavior requested by a `Location` navigation method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationBehavior {
    /// Add a new history visit, as for `href`, component setters, or `assign()`.
    Auto,
    /// Replace the current history visit, as for `replace()`.
    Replace,
    /// Reload the current URL without adding a history visit.
    Reload,
}

/// Thread-safe callback used to hand a script navigation request to the
/// browser's page loader.
pub type LocationNavigate = Arc<dyn Fn(Url, NavigationBehavior) + Send + Sync>;

/// Installs a `Location` snapshot without a browser navigation handler.
///
/// Use [`register_location_with_navigation`] when page scripts need to cause
/// real navigation. Without a handler, navigation requests are ignored.
pub fn register_location(realm: &mut Realm, url: Url) -> JsResult<()> {
    register_location_with_navigation(realm, url, Arc::new(|_, _| {})).map(|_| ())
}

/// Installs the current page's live `Location` object on `window` and as the
/// global `location` alias. An already registered `window` object is
/// extended in place; otherwise a new one is created. Mutating a URL component, assigning `href`, or
/// calling `assign()`, `replace()`, or `reload()` invokes `navigate` with the
/// parsed destination and requested history behavior.
///
/// The current browser has only a top-level browsing context, so
/// `ancestorOrigins` is empty. Cross-origin `Location` access checks and the
/// HTML Standard's unforgeable property descriptors are not implemented;
/// the JavaScript runtime does not yet model cross-origin windows or general
/// property attributes.
pub fn register_location_with_navigation(
    realm: &mut Realm,
    url: Url,
    navigate: LocationNavigate,
) -> JsResult<Object> {
    let current_url = Arc::new(Mutex::new(url));
    let location = location_object(Arc::clone(&current_url), Arc::clone(&navigate));
    // Reuse a window installed by `register_window*` so its other properties
    // survive; the accessor takes precedence over its data `location` slot.
    let window = match realm.registered_global_value("window") {
        Some(Value::Object(window)) => window,
        _ => Object::new(),
    };
    let location_for_window = location.clone();
    let window_url = Arc::clone(&current_url);
    let window_navigate = Arc::clone(&navigate);
    window.define_accessor(
        "location",
        Arc::new(move || Ok(Value::Object(location_for_window.clone()))),
        Some(Arc::new(move |value| {
            navigate_to_value(
                &window_url,
                &window_navigate,
                value,
                NavigationBehavior::Auto,
            )
        })),
    );
    realm.register_global_value("window", Value::Object(window))?;
    realm.register_global_value("location", Value::Object(location.clone()))?;
    let url = Arc::clone(&current_url);
    let handler = Arc::clone(&navigate);
    realm.register_global_setter(
        "location",
        Arc::new(move |value| navigate_to_value(&url, &handler, value, NavigationBehavior::Auto)),
    )?;
    Ok(location)
}

fn location_object(current_url: Arc<Mutex<Url>>, navigate: LocationNavigate) -> Object {
    let location = Object::new();
    for property in [
        "href", "origin", "protocol", "host", "hostname", "port", "pathname", "search", "hash",
    ] {
        let url = Arc::clone(&current_url);
        let getter: HostGetter = Arc::new(move || {
            let url = url.lock().unwrap_or_else(|poison| poison.into_inner());
            Ok(Value::String(component(&url, property)))
        });
        let setter = if property == "origin" {
            None
        } else {
            let url = Arc::clone(&current_url);
            let navigate = Arc::clone(&navigate);
            Some(
                Arc::new(move |value| set_component(&url, &navigate, property, value))
                    as HostSetter,
            )
        };
        location.define_accessor(property, getter, setter);
    }

    let ancestor_origins = Object::new();
    ancestor_origins.set("length", Value::Number(0.0));
    ancestor_origins.set_method("item", Arc::new(|_| Ok(Value::Null)));
    ancestor_origins.set_method("contains", Arc::new(|_| Ok(Value::Boolean(false))));
    let empty_ancestor_origins = ancestor_origins;
    location.define_accessor(
        "ancestorOrigins",
        Arc::new(move || Ok(Value::Object(empty_ancestor_origins.clone()))),
        None,
    );

    let url = Arc::clone(&current_url);
    let assign_navigate = Arc::clone(&navigate);
    let assign: HostFunction = Arc::new(move |arguments| {
        navigate_method(&url, &assign_navigate, arguments, NavigationBehavior::Auto)
    });
    location.set_method("assign", assign);

    let url = Arc::clone(&current_url);
    let replace_navigate = Arc::clone(&navigate);
    let replace: HostFunction = Arc::new(move |arguments| {
        navigate_method(
            &url,
            &replace_navigate,
            arguments,
            NavigationBehavior::Replace,
        )
    });
    location.set_method("replace", replace);

    let url = Arc::clone(&current_url);
    let reload_navigate = Arc::clone(&navigate);
    let reload: HostFunction = Arc::new(move |arguments| {
        let _ = arguments;
        let url = url
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        reload_navigate(url, NavigationBehavior::Reload);
        Ok(Value::Undefined)
    });
    location.set_method("reload", reload);

    let url = Arc::clone(&current_url);
    location.set_method(
        "toString",
        Arc::new(move |_arguments| {
            Ok(Value::String(
                url.lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .as_str()
                    .to_owned(),
            ))
        }),
    );
    let this_location = location.downgrade();
    location.set_method(
        "valueOf",
        Arc::new(move |_| {
            Ok(this_location
                .upgrade()
                .map_or(Value::Undefined, Value::Object))
        }),
    );
    location
}

fn component(url: &Url, property: &str) -> String {
    match property {
        "href" => url.as_str().to_owned(),
        "origin" => url.origin().ascii_serialization(),
        "protocol" => format!("{}:", url.scheme()),
        "host" => {
            let hostname = url.host().map_or_else(String::new, |host| host.to_string());
            url.port()
                .map_or(hostname.clone(), |port| format!("{hostname}:{port}"))
        }
        "hostname" => url.host_str().unwrap_or_default().to_owned(),
        "port" => url.port().map_or_else(String::new, |port| port.to_string()),
        "pathname" => url.path().to_owned(),
        "search" => url
            .query()
            .map_or_else(String::new, |query| format!("?{query}")),
        "hash" => url
            .fragment()
            .map_or_else(String::new, |fragment| format!("#{fragment}")),
        _ => String::new(),
    }
}

fn navigate_method(
    current_url: &Arc<Mutex<Url>>,
    navigate: &LocationNavigate,
    arguments: &[Value],
    behavior: NavigationBehavior,
) -> JsResult<Value> {
    let value = arguments
        .first()
        .ok_or_else(|| JsError::new("location navigation method expects a URL"))?;
    navigate_to_value(current_url, navigate, value.clone(), behavior)?;
    Ok(Value::Undefined)
}

fn navigate_to_value(
    current_url: &Arc<Mutex<Url>>,
    navigate: &LocationNavigate,
    value: Value,
    behavior: NavigationBehavior,
) -> JsResult<()> {
    let reference = value.to_string();
    let base = current_url
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let destination = base
        .join(&reference)
        .map_err(|error| JsError::new(format!("Invalid URL: {error}")))?;
    drop(base);
    navigate(destination, behavior);
    Ok(())
}

fn set_component(
    current_url: &Arc<Mutex<Url>>,
    navigate: &LocationNavigate,
    property: &str,
    value: Value,
) -> JsResult<()> {
    let value = value.to_string();
    let current = current_url
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let mut updated = current.clone();
    let valid = match property {
        "href" => match current.join(&value) {
            Ok(url) => {
                updated = url;
                true
            }
            Err(_) => return Err(JsError::new("Invalid URL")),
        },
        "protocol" => updated.set_scheme(value.trim_end_matches(':')).is_ok(),
        "host" | "hostname" => {
            let old_port = updated.port();
            if property == "hostname"
                && value.contains(':')
                && !(value.starts_with('[') && value.ends_with(']'))
            {
                false
            } else {
                let valid = updated.set_host(Some(&value)).is_ok();
                if valid
                    && property == "hostname"
                    && let Some(port) = old_port
                {
                    let _ = updated.set_port(Some(port));
                }
                valid
            }
        }
        "port" => {
            if value.is_empty() {
                updated.set_port(None).is_ok()
            } else if let Ok(port) = value.parse::<u16>() {
                updated.set_port(Some(port)).is_ok()
            } else {
                false
            }
        }
        "pathname" => {
            if updated.cannot_be_a_base() {
                return Ok(());
            }
            updated.set_path(&value);
            true
        }
        "search" => {
            let query = value.strip_prefix('?').unwrap_or(&value);
            updated.set_query((!query.is_empty()).then_some(query));
            true
        }
        "hash" => {
            let fragment = value.strip_prefix('#').unwrap_or(&value);
            updated.set_fragment((!fragment.is_empty()).then_some(fragment));
            true
        }
        _ => false,
    };
    drop(current);
    if valid {
        navigate(updated, NavigationBehavior::Auto);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn location_reads_live_url_and_component_setters_request_navigation() {
        let navigations = Arc::new(Mutex::new(Vec::new()));
        let received = Arc::clone(&navigations);
        let navigate: LocationNavigate = Arc::new(move |url, behavior| {
            received.lock().unwrap().push((url, behavior));
        });
        let mut realm = Realm::new();
        register_location_with_navigation(
            &mut realm,
            Url::parse("https://example.test:8443/a/b?q=one#part").unwrap(),
            navigate,
        )
        .unwrap();

        assert_eq!(
            realm.evaluate_script("window.location.href"),
            Ok(Value::String(
                "https://example.test:8443/a/b?q=one#part".into()
            ))
        );
        assert_eq!(
            realm.evaluate_script(
                "location.origin + location.pathname + location.search + location.hash"
            ),
            Ok(Value::String(
                "https://example.test:8443/a/b?q=one#part".into()
            ))
        );
        assert_eq!(
            realm.evaluate_script("window.location === location"),
            Ok(Value::Boolean(true))
        );
        assert_eq!(
            realm.evaluate_script("location + ''"),
            Ok(Value::String(
                "https://example.test:8443/a/b?q=one#part".into()
            ))
        );
        assert_eq!(
            realm.evaluate_script(
                "location.ancestorOrigins.length === 0 && location.ancestorOrigins.item(0) === null"
            ),
            Ok(Value::Boolean(true))
        );
        realm
            .evaluate_script("location.pathname = '/next'")
            .unwrap();
        realm
            .evaluate_script("window.location = '/window-target'")
            .unwrap();
        realm
            .evaluate_script("location = '/global-target'")
            .unwrap();
        assert_eq!(
            navigations.lock().unwrap().as_slice(),
            &[
                (
                    Url::parse("https://example.test:8443/next?q=one#part").unwrap(),
                    NavigationBehavior::Auto,
                ),
                (
                    Url::parse("https://example.test:8443/window-target").unwrap(),
                    NavigationBehavior::Auto,
                ),
                (
                    Url::parse("https://example.test:8443/global-target").unwrap(),
                    NavigationBehavior::Auto,
                ),
            ]
        );
    }

    #[test]
    fn location_navigation_methods_preserve_history_behavior() {
        let navigations = Arc::new(Mutex::new(Vec::new()));
        let received = Arc::clone(&navigations);
        let navigate: LocationNavigate = Arc::new(move |url, behavior| {
            received.lock().unwrap().push((url, behavior));
        });
        let mut realm = Realm::new();
        register_location_with_navigation(
            &mut realm,
            Url::parse("https://example.test/old").unwrap(),
            navigate,
        )
        .unwrap();

        realm.evaluate_script("location.assign('/new')").unwrap();
        realm.evaluate_script("location.replace('/final')").unwrap();
        realm.evaluate_script("location.reload()").unwrap();
        let navigations = navigations.lock().unwrap();
        assert_eq!(navigations.len(), 3);
        assert_eq!(navigations[0].1, NavigationBehavior::Auto);
        assert_eq!(navigations[1].1, NavigationBehavior::Replace);
        assert_eq!(navigations[2].1, NavigationBehavior::Reload);
    }
}
