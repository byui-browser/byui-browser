//! The script-visible Web Crypto API entry point.

use std::sync::Arc;

use getrandom::fill;
use js::{HostFunction, JsError, JsResult, Object, Realm, Value};

const MAX_RANDOM_VALUES_BYTES: usize = 65_536;

fn random_bytes(length: usize) -> JsResult<Vec<u8>> {
    let mut bytes = vec![0; length];
    fill(&mut bytes)
        .map_err(|error| JsError::runtime(format!("secure random source failed: {error}")))?;
    Ok(bytes)
}

fn get_random_values(arguments: &[Value]) -> JsResult<Value> {
    let Some(Value::Array(array)) = arguments.first() else {
        return Err(JsError::runtime(
            "crypto.getRandomValues() requires an array",
        ));
    };
    let length = match array.get("length") {
        Some(Value::Number(length)) if length.is_finite() && length >= 0.0 => length as usize,
        _ => 0,
    };
    if length > MAX_RANDOM_VALUES_BYTES {
        return Err(JsError::runtime(
            "crypto.getRandomValues() array exceeds the 65536-byte quota",
        ));
    }
    for (index, byte) in random_bytes(length)?.into_iter().enumerate() {
        array.set(index.to_string(), Value::Number(f64::from(byte)));
    }
    Ok(Value::Array(array.clone()))
}

fn random_uuid(arguments: &[Value]) -> JsResult<Value> {
    let _ = arguments;
    let mut bytes = random_bytes(16)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let uuid = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    );
    Ok(Value::String(uuid))
}

/// Registers the stable `window.crypto` object and its global alias.
///
/// `getRandomValues()` uses the operating system's cryptographically secure
/// random source and accepts this engine's ordinary JavaScript arrays because
/// typed arrays are not implemented yet. The Web Crypto specification requires
/// an integer typed array; callers should treat ordinary-array support as a
/// temporary engine limitation. `subtle` is present as a stable empty object,
/// but its cryptographic operations are not implemented yet.
pub fn register_crypto(realm: &mut Realm, window: &Object) -> JsResult<()> {
    let crypto = Object::new();
    let subtle = Object::new();
    crypto.set_read_only("subtle", Value::Object(subtle));
    let get_random_values: HostFunction = Arc::new(get_random_values);
    crypto.set_method("getRandomValues", get_random_values);
    let random_uuid: HostFunction = Arc::new(random_uuid);
    crypto.set_method("randomUUID", random_uuid);
    window.set_read_only("crypto", Value::Object(crypto.clone()));
    realm.register_global_value("crypto", Value::Object(crypto))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::get_random_values;
    use js::{Object, Value};

    #[test]
    fn get_random_values_enforces_the_spec_quota() {
        let array = Object::new();
        array.set("length", Value::Number(65_537.0));

        assert!(get_random_values(&[Value::Array(array)]).is_err());
    }
}
