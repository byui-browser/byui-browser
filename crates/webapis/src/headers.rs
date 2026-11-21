//! JavaScript binding for the Fetch `Headers` collection.

use std::sync::{Arc, Mutex};

use js::{Function, JsError, JsResult, Object, Realm, Value};
use net::Headers as HeaderList;

fn build(arguments: &[Value]) -> JsResult<Value> {
    let mut headers = HeaderList::new();
    if let Some(init) = arguments.first() {
        match init {
            Value::Null | Value::Undefined => {}
            Value::Array(pairs) => {
                let length = pairs
                    .get("length")
                    .and_then(|value| match value {
                        Value::Number(n) if n >= 0.0 => Some(n as usize),
                        _ => None,
                    })
                    .unwrap_or(0);
                for index in 0..length {
                    let Some(Value::Array(pair)) = pairs.get(&index.to_string()) else {
                        return Err(JsError::runtime(
                            "Headers initializer entries must be pairs",
                        ));
                    };
                    let name = pair
                        .get("0")
                        .ok_or_else(|| JsError::runtime("Header pair requires a name"))?
                        .to_string();
                    let value = pair
                        .get("1")
                        .ok_or_else(|| JsError::runtime("Header pair requires a value"))?
                        .to_string();
                    headers
                        .append_str(&name, &value)
                        .map_err(|error| JsError::runtime(error.to_string()))?;
                }
            }
            Value::Object(object) => {
                for name in object.keys() {
                    if let Some(value) = object.get(&name) {
                        headers
                            .append_str(&name, &value.to_string())
                            .map_err(|error| JsError::runtime(error.to_string()))?;
                    }
                }
            }
            _ => {
                return Err(JsError::runtime(
                    "Headers initializer must be an object or sequence",
                ));
            }
        }
    }
    let headers = Arc::new(Mutex::new(headers));
    let object = Object::new();
    for (method, operation) in [
        ("append", 0),
        ("set", 1),
        ("delete", 2),
        ("has", 3),
        ("get", 4),
    ] {
        let headers = headers.clone();
        object.set_method(
            method,
            Arc::new(move |args| {
                let name = args
                    .first()
                    .ok_or_else(|| JsError::runtime(format!("Headers.{method}() requires a name")))?
                    .to_string();
                match operation {
                    0 | 1 => {
                        let value = args
                            .get(1)
                            .ok_or_else(|| {
                                JsError::runtime(format!("Headers.{method}() requires a value"))
                            })?
                            .to_string();
                        let mut headers = headers.lock().unwrap();
                        let result = if operation == 0 {
                            headers.append_str(&name, &value)
                        } else {
                            headers.set_str(&name, &value)
                        };
                        result.map_err(|error| JsError::runtime(error.to_string()))?;
                        Ok(Value::Undefined)
                    }
                    2 => {
                        headers
                            .lock()
                            .unwrap()
                            .delete(&name)
                            .map_err(|error| JsError::runtime(error.to_string()))?;
                        Ok(Value::Undefined)
                    }
                    3 => Ok(Value::Boolean(headers.lock().unwrap().has(&name))),
                    _ => Ok(headers
                        .lock()
                        .unwrap()
                        .get(&name)
                        .map_or(Value::Null, Value::String)),
                }
            }),
        );
    }
    let headers_for_entries = headers;
    let for_each_this = object.clone();
    object.set_method(
        "forEach",
        Arc::new(move |args| {
            let Some(Value::Function(callback)) = args.first() else {
                return Err(JsError::runtime("Headers.forEach() requires a callback"));
            };
            let entries = headers_for_entries
                .lock()
                .unwrap()
                .iter()
                .collect::<Vec<_>>();
            for (name, value) in entries {
                callback.call(&[
                    Value::String(value),
                    Value::String(name.to_string()),
                    Value::Object(for_each_this.clone()),
                ])?;
            }
            Ok(Value::Undefined)
        }),
    );
    Ok(Value::Object(object))
}

/// Registers the JavaScript `Headers` global and its `window` alias.
pub(crate) fn register(realm: &mut Realm, window: &Object) -> JsResult<()> {
    let constructor = Function::from_host_constructor(
        "Headers",
        Arc::new(|_| Err(JsError::runtime("Headers constructor requires 'new'"))),
        Arc::new(build),
    );
    window.set_read_only("Headers", Value::Function(constructor.clone()));
    realm.register_global_value("Headers", Value::Function(constructor))
}
