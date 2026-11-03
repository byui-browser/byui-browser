//! Plain JavaScript objects: string-keyed property tables shared by reference.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use super::{Function, WeakFunction};
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
/// An object may have a prototype object. [`Object::get`] falls back to the
/// prototype chain when an object has no such own property, while
/// [`Object::set`] always writes an own property, so a write through an
/// instance shadows the prototype instead of changing it. Only the bytecode
/// VM creates prototypes (for `class` and `new`); [`Object::new`] makes an
/// object with none.
///
/// A class prototype reaches its class through `constructor` without owning
/// it, so a class and its prototype do not keep each other alive. The link is
/// not an own property: it is not listed by [`Object::keys`], and it reads as
/// missing once the class has been dropped.
///
/// Limitations: there is no built-in `Object.prototype`, so objects have no
/// inherited properties such as `toString`. There are no accessors,
/// attributes, or symbol keys. As with functions, there is no garbage
/// collector, so an object that refers to itself (directly or through other
/// objects) is never freed.
#[derive(Clone, Default)]
pub struct Object(Arc<ObjectData>);

#[derive(Default)]
struct ObjectData {
    properties: Mutex<Vec<(String, Value)>>,
    prototype: Mutex<Option<Object>>,
    constructor: Mutex<Option<WeakFunction>>,
}

/// A non-owning reference to an [`Object`], used to break reference cycles.
#[derive(Clone)]
pub(crate) struct WeakObject(Weak<ObjectData>);

impl WeakObject {
    /// Returns the object if something else still keeps it alive.
    pub(crate) fn upgrade(&self) -> Option<Object> {
        self.0.upgrade().map(Object)
    }
}

impl Object {
    /// Creates an empty object with a new identity and no prototype.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty object whose prototype is `prototype`.
    pub fn with_prototype(prototype: Option<Object>) -> Self {
        let object = Self::new();
        *object.prototype_slot() = prototype;
        object
    }

    fn properties(&self) -> MutexGuard<'_, Vec<(String, Value)>> {
        // Nothing panics while holding the lock, so a poisoned table is
        // still consistent.
        self.0
            .properties
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Returns a non-owning reference to this object.
    pub(crate) fn downgrade(&self) -> WeakObject {
        WeakObject(Arc::downgrade(&self.0))
    }

    /// Records the class this object is the prototype of, without owning it.
    pub(crate) fn set_constructor(&self, constructor: &Function) {
        *self
            .0
            .constructor
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(constructor.downgrade());
    }

    fn prototype_slot(&self) -> MutexGuard<'_, Option<Object>> {
        self.0
            .prototype
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Returns the object's prototype, or `None` if it has none.
    pub fn prototype(&self) -> Option<Object> {
        self.prototype_slot().clone()
    }

    /// Replaces the object's prototype, returning `false` and changing
    /// nothing if that would make the prototype chain a cycle.
    pub fn set_prototype(&self, prototype: Option<Object>) -> bool {
        let mut link = prototype.clone();
        while let Some(object) = link {
            if object == *self {
                return false;
            }
            link = object.prototype();
        }
        *self.prototype_slot() = prototype;
        true
    }

    /// Returns a clone of the value stored under `key`, looking first at the
    /// object's own properties and then along its prototype chain, or `None`
    /// if no object on the chain has the property.
    pub fn get(&self, key: &str) -> Option<Value> {
        let mut current = self.clone();
        loop {
            if let Some(value) = current.get_own(key) {
                return Some(value);
            }
            current = current.prototype()?;
        }
    }

    /// Returns a clone of the value stored under `key` on this object
    /// itself, ignoring the prototype chain.
    pub fn get_own(&self, key: &str) -> Option<Value> {
        let own = self
            .properties()
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone());
        if own.is_none() && key == "constructor" {
            let link = self
                .0
                .constructor
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            return link.and_then(|link| link.upgrade()).map(Value::Function);
        }
        own
    }

    /// Stores `value` under `key` as an own property, replacing any existing
    /// own value in place or appending a new property at the end of the
    /// insertion order. The prototype chain is never modified.
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

    /// Returns whether the object itself, not its prototype chain, has a
    /// property named `key`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.properties().iter().any(|(name, _)| name == key)
    }

    /// Returns the own property names in insertion order.
    pub fn keys(&self) -> Vec<String> {
        self.properties()
            .iter()
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Returns the number of own properties.
    pub fn len(&self) -> usize {
        self.properties().len()
    }

    /// Returns whether the object has no own properties.
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
