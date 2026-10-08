//! Plain JavaScript objects: string-keyed property tables shared by reference.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::Function;
use crate::{HostFunction, Value};

/// A JavaScript object value: a table of owned string keys to owned values.
///
/// `Object` is a handle. Cloning it is cheap and yields another reference to
/// the same properties, so a property set through one clone is visible
/// through every other, exactly as when a script copies an object reference.
/// Clones compare equal with `==` and `===`; separately created objects
/// never do, even with identical properties.
///
/// Properties are kept in insertion order and are always plain, writable
/// data properties. Because the table is shared, methods take `&self` and
/// lock it internally; the lock is never held while script code runs.
///
/// Limitations: there are no prototypes, so objects have no inherited
/// properties such as `toString`. There are no accessors, attributes, or
/// symbol keys. As with functions, there is no garbage collector, so an
/// object that refers to itself (directly or through other objects) is never
/// freed.
#[derive(Clone, Default)]
pub struct Object(Arc<Mutex<Vec<(String, Value)>>>);

impl Object {
    /// Creates an empty object with a new identity.
    pub fn new() -> Self {
        Self::default()
    }

    fn properties(&self) -> MutexGuard<'_, Vec<(String, Value)>> {
        // Nothing panics while holding the lock, so a poisoned table is
        // still consistent.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Returns a clone of the value stored under `key`, or `None` if the
    /// object has no such property.
    pub fn get(&self, key: &str) -> Option<Value> {
        self.properties()
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    }

    /// Stores `value` under `key`, replacing any existing value in place or
    /// appending a new property at the end of the insertion order.
    pub fn set(&self, key: impl Into<String>, value: Value) {
        let key = key.into();
        let mut properties = self.properties();
        match properties.iter_mut().find(|(name, _)| *name == key) {
            Some((_, existing)) => *existing = value,
            None => properties.push((key, value)),
        }
    }

    /// Stores a host function under `name` as a method that scripts can call
    /// as `object.name(...)`. The function receives only the call's
    /// arguments; it is not given the object as `this`.
    pub fn set_method(&self, name: impl Into<String>, function: HostFunction) {
        let name = name.into();
        let function = Function::from_host(name.clone(), function);
        self.set(name, Value::Function(function));
    }

    /// Returns whether the object has a property named `key`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.properties().iter().any(|(name, _)| name == key)
    }

    /// Returns the property names in insertion order.
    pub fn keys(&self) -> Vec<String> {
        self.properties()
            .iter()
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Returns the number of properties.
    pub fn len(&self) -> usize {
        self.properties().len()
    }

    /// Returns whether the object has no properties.
    pub fn is_empty(&self) -> bool {
        self.properties().is_empty()
    }
}

impl PartialEq for Object {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Lists only the property names, so objects that refer to themselves do not
/// recurse forever.
impl fmt::Debug for Object {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Object")
            .field("keys", &self.keys())
            .finish()
    }
}

/// The JavaScript string form of a plain object.
impl fmt::Display for Object {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[object Object]")
    }
}
