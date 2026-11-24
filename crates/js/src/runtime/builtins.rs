//! ECMAScript built-ins that do not depend on browser host APIs.

use std::sync::{Arc, Mutex, OnceLock};

use crate::{Function, JsError, JsResult, Object, Value};

type Entries = Arc<Mutex<Vec<(Value, Value)>>>;
type Values = Arc<Mutex<Vec<Value>>>;

/// Installs the standard-library objects supported by the engine.
pub(super) fn install(state: &super::GlobalState) {
    super::core::install(state);
    super::primitives::install(state);
    super::errors::install(state);

    state.install_host("Map", Value::Function(map_constructor()));
    state.install_host("Set", Value::Function(set_constructor()));
    state.install_host("WeakMap", Value::Function(map_constructor()));
    state.install_host("WeakSet", Value::Function(set_constructor()));
    state.install_host("Symbol", Value::Function(symbol_constructor()));
    state.install_host("ArrayBuffer", Value::Function(array_buffer_constructor()));
    state.install_host("Uint8Array", Value::Function(uint8_array_constructor()));
    for name in [
        "Int8Array",
        "Uint16Array",
        "Uint32Array",
        "Uint8ClampedArray",
    ] {
        state.install_host(name, Value::Function(typed_array_constructor(name)));
    }
    state.install_host("TextEncoder", Value::Function(text_encoder_constructor()));

    state.install_host("Reflect", Value::Object(reflect_namespace()));
    state.install_global_this();
}

fn reflect_namespace() -> Object {
    let reflect = Object::new();
    reflect.set_method(
        "get",
        Arc::new(|args| {
            let object = object_argument(args, 0)?;
            let key = property_key(args.get(1).unwrap_or(&Value::Undefined));
            Ok(object.get_property(&key)?.unwrap_or(Value::Undefined))
        }),
    );
    reflect.set_method(
        "set",
        Arc::new(|args| {
            let object = object_argument(args, 0)?;
            let key = property_key(args.get(1).unwrap_or(&Value::Undefined));
            let value = args.get(2).cloned().unwrap_or(Value::Undefined);
            object.set_property(key, value)?;
            Ok(Value::Boolean(true))
        }),
    );
    reflect.set_method(
        "has",
        Arc::new(|args| {
            let object = object_argument(args, 0)?;
            let key = property_key(args.get(1).unwrap_or(&Value::Undefined));
            Ok(Value::Boolean(object.get_property(&key)?.is_some()))
        }),
    );
    reflect.set_method(
        "deleteProperty",
        Arc::new(|args| {
            let object = object_argument(args, 0)?;
            let key = property_key(args.get(1).unwrap_or(&Value::Undefined));
            let existed = object.get_property(&key)?.is_some();
            object.remove(&key);
            Ok(Value::Boolean(existed))
        }),
    );
    reflect.set_method(
        "construct",
        Arc::new(|args| {
            let Some(Value::Function(constructor)) = args.first() else {
                return Err(JsError::new(
                    "Reflect.construct target is not a constructor",
                ));
            };
            let arguments = match args.get(1) {
                None | Some(Value::Undefined) => Vec::new(),
                Some(Value::Array(array)) => (0..array
                    .get("length")
                    .and_then(|v| match v {
                        Value::Number(n) => Some(n as usize),
                        _ => None,
                    })
                    .unwrap_or(0))
                    .map(|i| array.get(&i.to_string()).unwrap_or(Value::Undefined))
                    .collect(),
                _ => {
                    return Err(JsError::new(
                        "Reflect.construct argumentsList must be an array",
                    ));
                }
            };
            constructor.construct(&arguments)
        }),
    );
    reflect.set_method(
        "decorate",
        Arc::new(|args| {
            let Some(Value::Array(decorators)) = args.first() else {
                return Err(JsError::new(
                    "Reflect.decorate requires an array of decorators",
                ));
            };
            let target = object_argument(args, 1)?;
            let key = args.get(2).map(property_key);
            let descriptor = args.get(3).cloned().unwrap_or(Value::Undefined);
            let len = decorators
                .get("length")
                .and_then(|v| match v {
                    Value::Number(n) => Some(n as usize),
                    _ => None,
                })
                .unwrap_or(0);
            let mut result = Value::Object(target.clone());
            for index in (0..len).rev() {
                let Some(Value::Function(decorator)) = decorators.get(&index.to_string()) else {
                    return Err(JsError::new("Decorator is not callable"));
                };
                let call_args = match &key {
                    Some(key) => vec![
                        result.clone(),
                        Value::String(key.clone()),
                        descriptor.clone(),
                    ],
                    None => vec![result.clone()],
                };
                let decorated = decorator.call(&call_args)?;
                if !matches!(decorated, Value::Undefined | Value::Null) {
                    result = decorated;
                }
            }
            Ok(result)
        }),
    );
    reflect.set_method(
        "metadata",
        Arc::new(|args| {
            let metadata_key = property_key(args.first().unwrap_or(&Value::Undefined));
            let metadata_value = args.get(1).cloned().unwrap_or(Value::Undefined);
            Ok(Value::Function(Function::from_host(
                "metadata",
                Arc::new(move |decorator_args| {
                    let target = object_argument(decorator_args, 0)?;
                    let property = decorator_args.get(1).map(property_key).unwrap_or_default();
                    let bag_key = format!("__metadata:{property}:{metadata_key}");
                    target.set_property(bag_key, metadata_value.clone())?;
                    Ok(Value::Undefined)
                }),
            )))
        }),
    );
    reflect
}

/// Returns a fresh standalone value for a standard global. The VM uses this
/// for its intentionally short-lived, non-persistent global environment.
pub(crate) fn value(name: &str) -> Option<Value> {
    if let Some(value) = super::core::value(name) {
        return Some(value);
    }
    match name {
        "String" => Some(Value::Function(
            super::primitives::string_constructor_value(),
        )),
        "Number" => Some(Value::Function(
            super::primitives::number_constructor_value(),
        )),
        "Map" => Some(Value::Function(map_constructor())),
        "Set" => Some(Value::Function(set_constructor())),
        "WeakMap" => Some(Value::Function(map_constructor())),
        "WeakSet" => Some(Value::Function(set_constructor())),
        "Symbol" => Some(Value::Function(symbol_constructor())),
        "ArrayBuffer" => Some(Value::Function(array_buffer_constructor())),
        "Uint8Array" => Some(Value::Function(uint8_array_constructor())),
        "Int8Array" => Some(Value::Function(typed_array_constructor("Int8Array"))),
        "Uint16Array" => Some(Value::Function(typed_array_constructor("Uint16Array"))),
        "Uint32Array" => Some(Value::Function(typed_array_constructor("Uint32Array"))),
        "Uint8ClampedArray" => Some(Value::Function(typed_array_constructor(
            "Uint8ClampedArray",
        ))),
        "TextEncoder" => Some(Value::Function(text_encoder_constructor())),
        "Reflect" => Some(Value::Object(reflect_namespace())),
        _ => super::global_functions::value(name).or_else(|| super::errors::value(name)),
    }
}

fn typed_array_constructor(name: &'static str) -> Function {
    Function::from_host_constructor(
        name,
        Arc::new(move |_| Err(JsError::new(format!("Constructor {name} requires 'new'")))),
        Arc::new(move |args| {
            let count = match args.first() {
                Some(Value::Array(array)) => array
                    .get("length")
                    .and_then(|v| match v {
                        Value::Number(n) => Some(n as usize),
                        _ => None,
                    })
                    .unwrap_or(0),
                Some(Value::Number(n)) => checked_length(*n)?,
                _ => 0,
            };
            let bytes_per_element = match name {
                "Uint16Array" => 2,
                "Uint32Array" => 4,
                _ => 1,
            };
            let data = Arc::new(Mutex::new(vec![0.0; count]));
            if let Some(Value::Array(array)) = args.first() {
                let mut target = data.lock().unwrap();
                for (i, slot) in target.iter_mut().enumerate() {
                    *slot = array
                        .get(&i.to_string())
                        .map(|v| numeric_value(&v))
                        .unwrap_or(0.0);
                    *slot = convert_typed(name, *slot);
                }
            }
            let object = Object::new();
            object.set_read_only("length", Value::Number(count as f64));
            object.set_read_only(
                "byteLength",
                Value::Number((count * bytes_per_element) as f64),
            );
            let read = data.clone();
            let write = data.clone();
            object.set_property_hooks(
                Arc::new(move |key| {
                    key.parse::<usize>()
                        .ok()
                        .and_then(|i| read.lock().ok()?.get(i).copied())
                        .map(|n| Ok(Some(Value::Number(n))))
                        .unwrap_or(Ok(None))
                }),
                Arc::new(move |key, value| {
                    let Ok(i) = key.parse::<usize>() else {
                        return Ok(false);
                    };
                    let mut values = write.lock().unwrap();
                    let Some(slot) = values.get_mut(i) else {
                        return Err(JsError::new("typed-array index out of bounds"));
                    };
                    *slot = convert_typed(name, numeric_value(&value));
                    Ok(true)
                }),
                Arc::new(move || Ok((0..count).map(|i| i.to_string()).collect())),
            );
            Ok(Value::Object(object))
        }),
    )
}

fn numeric_value(value: &Value) -> f64 {
    match value {
        Value::Number(n) => *n,
        Value::Boolean(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        Value::Null => 0.0,
        _ => 0.0,
    }
}

fn convert_typed(name: &str, number: f64) -> f64 {
    if !number.is_finite() {
        return 0.0;
    }
    match name {
        "Int8Array" => (number as i64 as i8) as f64,
        "Uint16Array" => number.rem_euclid(65536.0).trunc(),
        "Uint32Array" => number.rem_euclid(4294967296.0).trunc(),
        "Uint8ClampedArray" => number.round_ties_even().clamp(0.0, 255.0),
        _ => number.rem_euclid(256.0).trunc(),
    }
}

fn text_encoder_constructor() -> Function {
    Function::from_host_constructor(
        "TextEncoder",
        Arc::new(|_| Err(JsError::new("Constructor TextEncoder requires 'new'"))),
        Arc::new(|_| {
            let object = Object::new();
            object.set_read_only("encoding", Value::String("utf-8".into()));
            object.set_method(
                "encode",
                Arc::new(|args| {
                    let text = match args.first() {
                        Some(Value::String(s)) => s.clone(),
                        None | Some(Value::Undefined) => String::new(),
                        Some(v) => format!("{v:?}"),
                    };
                    let source = Object::new();
                    source.set_property("length", Value::Number(text.len() as f64))?;
                    for (index, byte) in text.as_bytes().iter().enumerate() {
                        source.set_property(index.to_string(), Value::Number(f64::from(*byte)))?;
                    }
                    uint8_array_constructor().construct(&[Value::Array(source)])
                }),
            );
            Ok(Value::Object(object))
        }),
    )
}

fn array_buffer_constructor() -> Function {
    Function::from_host_constructor(
        "ArrayBuffer",
        Arc::new(|_| Err(JsError::new("Constructor ArrayBuffer requires 'new'"))),
        Arc::new(|args| {
            let length = byte_length(args.first().unwrap_or(&Value::Number(0.0)))?;
            let bytes = Arc::new(Mutex::new(vec![0; length]));
            let object = Object::new();
            object.set_read_only("byteLength", Value::Number(length as f64));
            let view = bytes.clone();
            object.set_method(
                "slice",
                Arc::new(move |args| {
                    let source = view.lock().unwrap();
                    let start = index_argument(args.first(), 0, source.len());
                    let end = index_argument(args.get(1), source.len(), source.len());
                    let copied = source[start.min(source.len())
                        ..end.min(source.len()).max(start.min(source.len()))]
                        .to_vec();
                    let result = Object::new();
                    result.set_read_only("byteLength", Value::Number(copied.len() as f64));
                    Ok(Value::Object(result))
                }),
            );
            Ok(Value::Object(object))
        }),
    )
}

fn uint8_array_constructor() -> Function {
    Function::from_host_constructor(
        "Uint8Array",
        Arc::new(|_| Err(JsError::new("Constructor Uint8Array requires 'new'"))),
        Arc::new(|args| {
            let length = match args.first() {
                Some(Value::Array(array)) => array
                    .get("length")
                    .and_then(|value| match value {
                        Value::Number(n) => Some(n as usize),
                        _ => None,
                    })
                    .unwrap_or(0),
                Some(Value::Number(number)) => checked_length(*number)?,
                _ => 0,
            };
            let bytes = Arc::new(Mutex::new(vec![0_u8; length]));
            if let Some(Value::Array(array)) = args.first() {
                let mut target = bytes.lock().unwrap();
                for (index, byte) in target.iter_mut().enumerate() {
                    if let Some(Value::Number(value)) = array.get(&index.to_string()) {
                        *byte = to_byte(value);
                    }
                }
            }
            let object = Object::new();
            object.set_read_only("length", Value::Number(length as f64));
            object.set_read_only("byteLength", Value::Number(length as f64));
            let read = bytes.clone();
            let write = bytes.clone();
            object.set_property_hooks(
                Arc::new(move |key| {
                    let Ok(index) = key.parse::<usize>() else {
                        return Ok(None);
                    };
                    Ok(read
                        .lock()
                        .unwrap()
                        .get(index)
                        .map(|byte| Value::Number(f64::from(*byte))))
                }),
                Arc::new(move |key, value| {
                    let Ok(index) = key.parse::<usize>() else {
                        return Ok(false);
                    };
                    let mut bytes = write.lock().unwrap();
                    let Some(byte) = bytes.get_mut(index) else {
                        return Err(JsError::new("typed-array index out of bounds"));
                    };
                    let Value::Number(number) = value else {
                        return Err(JsError::new("typed-array value is not a number"));
                    };
                    *byte = to_byte(number);
                    Ok(true)
                }),
                Arc::new(move || Ok((0..length).map(|index| index.to_string()).collect())),
            );
            Ok(Value::Object(object))
        }),
    )
}

fn checked_length(number: f64) -> JsResult<usize> {
    if !number.is_finite() || number < 0.0 || number.fract() != 0.0 || number > usize::MAX as f64 {
        return Err(JsError::new("invalid typed-array length"));
    }
    Ok(number as usize)
}

fn to_byte(number: f64) -> u8 {
    if !number.is_finite() {
        return 0;
    }
    (number.rem_euclid(256.0)) as u8
}

fn byte_length(value: &Value) -> JsResult<usize> {
    match value {
        Value::Number(number) => checked_length(*number),
        _ => Err(JsError::new("invalid ArrayBuffer length")),
    }
}

fn index_argument(value: Option<&Value>, default: usize, length: usize) -> usize {
    match value {
        Some(Value::Number(number)) if *number >= 0.0 => (*number as usize).min(length),
        _ => default.min(length),
    }
}

fn map_constructor() -> Function {
    let create = Arc::new(|args: &[Value]| {
        let entries: Entries = Arc::new(Mutex::new(Vec::new()));
        if let Some(Value::Array(array)) = args.first() {
            let length = array
                .get("length")
                .and_then(|value| match value {
                    Value::Number(n) => Some(n as usize),
                    _ => None,
                })
                .unwrap_or(0);
            for index in 0..length {
                if let Some(Value::Array(pair)) = array.get(&index.to_string()) {
                    let key = pair.get("0").unwrap_or(Value::Undefined);
                    let value = pair.get("1").unwrap_or(Value::Undefined);
                    entries.lock().unwrap().push((key, value));
                }
            }
        }
        let object = Object::new();
        let self_reference = object.downgrade();
        let length = entries.clone();
        object.define_accessor(
            "size",
            Arc::new(move || Ok(Value::Number(length.lock().unwrap().len() as f64))),
            None,
        );
        let get_entries = entries.clone();
        object.set_method(
            "set",
            Arc::new(move |args| {
                let key = args.first().cloned().unwrap_or(Value::Undefined);
                let value = args.get(1).cloned().unwrap_or(Value::Undefined);
                let mut entries = get_entries.lock().unwrap();
                if let Some((_, current)) = entries
                    .iter_mut()
                    .find(|(current, _)| same_value_zero(current, &key))
                {
                    *current = value;
                } else {
                    entries.push((key, value));
                }
                Ok(self_reference
                    .upgrade()
                    .map(Value::Object)
                    .unwrap_or(Value::Undefined))
            }),
        );
        let get_entries = entries.clone();
        object.set_method(
            "get",
            Arc::new(move |args| {
                let key = args.first().cloned().unwrap_or(Value::Undefined);
                Ok(get_entries
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|(current, _)| same_value_zero(current, &key))
                    .map(|(_, value)| value.clone())
                    .unwrap_or(Value::Undefined))
            }),
        );
        let has_entries = entries.clone();
        object.set_method(
            "has",
            Arc::new(move |args| {
                let key = args.first().cloned().unwrap_or(Value::Undefined);
                Ok(Value::Boolean(
                    has_entries
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|(current, _)| same_value_zero(current, &key)),
                ))
            }),
        );
        let delete_entries = entries.clone();
        object.set_method(
            "delete",
            Arc::new(move |args| {
                let key = args.first().cloned().unwrap_or(Value::Undefined);
                let mut entries = delete_entries.lock().unwrap();
                let before = entries.len();
                entries.retain(|(current, _)| !same_value_zero(current, &key));
                Ok(Value::Boolean(before != entries.len()))
            }),
        );
        let clear_entries = entries.clone();
        object.set_method(
            "clear",
            Arc::new(move |_| {
                clear_entries.lock().unwrap().clear();
                Ok(Value::Undefined)
            }),
        );
        Ok(Value::Object(object))
    });
    Function::from_host_constructor(
        "Map",
        Arc::new(|_| Err(JsError::new("Constructor Map requires 'new'"))),
        create,
    )
}

fn set_constructor() -> Function {
    let create = Arc::new(|args: &[Value]| {
        let values: Values = Arc::new(Mutex::new(Vec::new()));
        if let Some(Value::Array(array)) = args.first() {
            let length = array
                .get("length")
                .and_then(|value| match value {
                    Value::Number(n) => Some(n as usize),
                    _ => None,
                })
                .unwrap_or(0);
            let mut target = values.lock().unwrap();
            for index in 0..length {
                target.push(array.get(&index.to_string()).unwrap_or(Value::Undefined));
            }
        }
        let object = Object::new();
        let self_reference = object.downgrade();
        let size = values.clone();
        object.define_accessor(
            "size",
            Arc::new(move || Ok(Value::Number(size.lock().unwrap().len() as f64))),
            None,
        );
        let add_values = values.clone();
        object.set_method(
            "add",
            Arc::new(move |args| {
                let value = args.first().cloned().unwrap_or(Value::Undefined);
                let mut values = add_values.lock().unwrap();
                if !values
                    .iter()
                    .any(|current| same_value_zero(current, &value))
                {
                    values.push(value);
                }
                Ok(self_reference
                    .upgrade()
                    .map(Value::Object)
                    .unwrap_or(Value::Undefined))
            }),
        );
        let has_values = values.clone();
        object.set_method(
            "has",
            Arc::new(move |args| {
                let value = args.first().cloned().unwrap_or(Value::Undefined);
                Ok(Value::Boolean(
                    has_values
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|current| same_value_zero(current, &value)),
                ))
            }),
        );
        let delete_values = values.clone();
        object.set_method(
            "delete",
            Arc::new(move |args| {
                let value = args.first().cloned().unwrap_or(Value::Undefined);
                let mut values = delete_values.lock().unwrap();
                let before = values.len();
                values.retain(|current| !same_value_zero(current, &value));
                Ok(Value::Boolean(before != values.len()))
            }),
        );
        let clear_values = values.clone();
        object.set_method(
            "clear",
            Arc::new(move |_| {
                clear_values.lock().unwrap().clear();
                Ok(Value::Undefined)
            }),
        );
        Ok(Value::Object(object))
    });
    Function::from_host_constructor(
        "Set",
        Arc::new(|_| Err(JsError::new("Constructor Set requires 'new'"))),
        create,
    )
}

// Well-known symbols share identity across constructors and realms.
// Limitation: like Symbol(), symbols are object handles rather than a distinct
// primitive variant. The well-known iterator key is supported by property
// access; automatic resource disposal is not modeled.
fn symbol_constructor() -> Function {
    static DISPOSE: OnceLock<Object> = OnceLock::new();
    static TO_PRIMITIVE: OnceLock<Object> = OnceLock::new();
    static ASYNC_DISPOSE: OnceLock<Object> = OnceLock::new();
    static ITERATOR: OnceLock<Object> = OnceLock::new();
    let function = Function::from_host(
        "Symbol",
        Arc::new(|args| {
            let symbol = Object::new();
            if let Some(description) = args.first() {
                symbol.set("description", description.clone());
            }
            Ok(Value::Object(symbol))
        }),
    );
    let dispose = DISPOSE.get_or_init(|| {
        let symbol = Object::new();
        symbol.set_read_only("description", Value::String("Symbol.dispose".into()));
        symbol
    });
    let to_primitive = TO_PRIMITIVE.get_or_init(|| {
        let symbol = Object::new();
        symbol.set_read_only("description", Value::String("Symbol.toPrimitive".into()));
        symbol
    });
    let async_dispose = ASYNC_DISPOSE.get_or_init(|| {
        let symbol = Object::new();
        symbol.set_read_only("description", Value::String("Symbol.asyncDispose".into()));
        symbol
    });
    let iterator = ITERATOR.get_or_init(|| {
        let symbol = Object::new();
        symbol.set_read_only("description", Value::String("Symbol.iterator".into()));
        symbol
    });
    match &*function.0 {
        super::FunctionKind::Host { properties, .. } => {
            properties.set_read_only("dispose", Value::Object(dispose.clone()));
            properties.set_read_only("toPrimitive", Value::Object(to_primitive.clone()));
            properties.set_read_only("asyncDispose", Value::Object(async_dispose.clone()));
            properties.set_read_only("iterator", Value::Object(iterator.clone()));
            let prototype = Object::new();
            properties.set_read_only("prototype", Value::Object(prototype));
        }
        _ => unreachable!(),
    }
    function
}

fn object_argument(args: &[Value], index: usize) -> JsResult<Object> {
    match args.get(index) {
        Some(Value::Object(object) | Value::Array(object)) => Ok(object.clone()),
        _ => Err(JsError::new("Reflect target is not an object")),
    }
}

fn property_key(value: &Value) -> String {
    value.to_string()
}

fn same_value_zero(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) if a.is_nan() && b.is_nan() => true,
        _ => left == right,
    }
}
