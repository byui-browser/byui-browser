//! Plain JavaScript objects: string-keyed property tables shared by reference.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use super::{Function, WeakFunction};
use crate::{HostFunction, JsResult, Value};

/// A thread-safe host getter for a JavaScript object property.
pub type HostGetter = Arc<dyn Fn() -> JsResult<Value> + Send + Sync>;
/// A thread-safe host setter for a JavaScript object property.
pub type HostSetter = Arc<dyn Fn(Value) -> JsResult<()> + Send + Sync>;
/// A thread-safe host hook for a dynamic JavaScript property read.
pub type HostPropertyGetter = Arc<dyn Fn(&str) -> JsResult<Option<Value>> + Send + Sync>;
/// A thread-safe host hook for a dynamic JavaScript property write.
pub type HostPropertySetter = Arc<dyn Fn(&str, Value) -> JsResult<bool> + Send + Sync>;
/// A thread-safe host hook for dynamic JavaScript property names.
pub type HostPropertyKeys = Arc<dyn Fn() -> JsResult<Vec<String>> + Send + Sync>;

/// A JavaScript object value: a table of owned string keys to owned values.
///
/// `Object` is a handle. Cloning it is cheap and yields another reference to
/// the same properties, so a property set through one clone is visible
/// through every other, exactly as when a script copies an object reference.
/// Clones compare equal with `==` and `===`; separately created objects
/// never do, even with identical properties.
///
/// Data properties and host accessors are kept in insertion order. Data
/// properties are writable unless defined with [`Object::set_read_only`], or
/// replaceable when defined with [`Object::set_replaceable_read_only`].
/// Accessor properties run their host callbacks without holding object locks.
/// Because the table is shared, methods take `&self` and lock it internally;
/// the lock is never held while script code runs.
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
/// Limitations: there is no built-in `Object.prototype`, property attributes,
/// or symbol keys. As with functions, there is no garbage collector, so an
/// object that refers to itself (directly or through other objects) is never
/// freed.
#[derive(Clone, Default)]
pub struct Object(Arc<ObjectData>);

#[derive(Default)]
struct ObjectData {
    properties: Mutex<Vec<(String, Value)>>,
    accessors: Mutex<Vec<(String, HostGetter, Option<HostSetter>)>>,
    read_only_properties: Mutex<HashSet<String>>,
    replaceable_properties: Mutex<HashSet<String>>,
    prototype: Mutex<Option<Object>>,
    constructor: Mutex<Option<WeakFunction>>,
    property_getter: Mutex<Option<HostPropertyGetter>>,
    property_setter: Mutex<Option<HostPropertySetter>>,
    property_keys: Mutex<Option<HostPropertyKeys>>,
    /// For the object a `catch` clause receives for an engine error: the
    /// error itself, restored if the object is thrown again unchanged.
    origin: Mutex<Option<crate::JsError>>,
}

/// A non-owning reference to an [`Object`], used to break reference cycles.
#[derive(Clone)]
pub struct WeakObject(Weak<ObjectData>);

impl WeakObject {
    /// Returns the object if something else still keeps it alive.
    /// Returns the object if another strong handle still keeps it alive.
    pub fn upgrade(&self) -> Option<Object> {
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

    /// Records the engine error this object was made from.
    pub(crate) fn set_origin(&self, error: crate::JsError) {
        *self.0.origin.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
    }

    /// Returns the engine error this object was made from, if any.
    pub(crate) fn origin(&self) -> Option<crate::JsError> {
        self.0
            .origin
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
    /// Creates a non-owning reference to this object.
    pub fn downgrade(&self) -> WeakObject {
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
    /// Host getter failures are suppressed here; use [`Object::get_property`]
    /// when the caller needs those errors.
    pub fn get(&self, key: &str) -> Option<Value> {
        self.get_property(key).ok().flatten()
    }

    /// Reads a property, invoking a host accessor when one is defined.
    ///
    /// Unlike [`Object::get`], this method can return errors produced by a
    /// host getter. The JavaScript runtime uses it for script property reads.
    pub fn get_property(&self, key: &str) -> JsResult<Option<Value>> {
        let mut current = self.clone();
        loop {
            let accessor = current
                .0
                .accessors
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .find(|(name, _, _)| name == key)
                .map(|(_, getter, _)| Arc::clone(getter));
            if let Some(getter) = accessor {
                return getter().map(Some);
            }
            if let Some(value) = current.get_own(key) {
                return Ok(Some(value));
            }
            if let Some(getter) = current
                .0
                .property_getter
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
                && let Some(value) = getter(key)?
            {
                return Ok(Some(value));
            }
            let Some(prototype) = current.prototype() else {
                return Ok(None);
            };
            current = prototype;
        }
    }

    /// Defines a host-backed property getter and optional setter.
    ///
    /// Host callbacks run without holding the object's property locks. A
    /// setter error is propagated to the JavaScript assignment expression.
    pub fn define_accessor(
        &self,
        name: impl Into<String>,
        getter: HostGetter,
        setter: Option<HostSetter>,
    ) {
        let name = name.into();
        self.set_own_accessor(name, getter, setter);
    }

    fn set_own_accessor(&self, name: String, getter: HostGetter, setter: Option<HostSetter>) {
        let mut accessors = self
            .0
            .accessors
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some((_, old_getter, old_setter)) =
            accessors.iter_mut().find(|(key, _, _)| key == &name)
        {
            *old_getter = getter;
            *old_setter = setter;
        } else {
            accessors.push((name, getter, setter));
        }
    }

    /// Assigns a property, invoking a host setter when defined.
    pub fn set_property(&self, key: impl Into<String>, value: Value) -> JsResult<()> {
        let key = key.into();
        let accessor = self
            .0
            .accessors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(name, _, _)| name == &key)
            .map(|(_, _, setter)| setter.clone());
        if let Some(setter) = accessor {
            return match setter {
                Some(setter) => setter(value),
                None => Ok(()),
            };
        }
        if !self.contains_key(&key)
            && let Some(setter) = self
                .0
                .property_setter
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
            && setter(&key, value.clone())?
        {
            return Ok(());
        }
        self.set(key, value);
        Ok(())
    }

    /// Installs hooks for properties backed by an external host object.
    pub fn set_property_hooks(
        &self,
        getter: HostPropertyGetter,
        setter: HostPropertySetter,
        keys: HostPropertyKeys,
    ) {
        *self
            .0
            .property_getter
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(getter);
        *self
            .0
            .property_setter
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(setter);
        *self
            .0
            .property_keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(keys);
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
        let is_replaceable = self
            .0
            .replaceable_properties
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&key);
        if is_replaceable {
            self.0
                .read_only_properties
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&key);
        } else if self
            .0
            .read_only_properties
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&key)
        {
            return;
        }
        match properties.iter_mut().find(|(name, _)| *name == key) {
            Some((_, existing)) => *existing = value,
            None => properties.push((key, value)),
        }
    }

    /// Defines a non-writable own data property under `key`, replacing any
    /// existing own value in place or appending a new property at the end of
    /// the insertion order. Later calls to [`Object::set`] leave this value
    /// unchanged. JavaScript strict-mode assignment errors are not modeled.
    pub fn set_read_only(&self, key: impl Into<String>, value: Value) {
        let key = key.into();
        let mut properties = self.properties();
        match properties.iter_mut().find(|(name, _)| *name == key) {
            Some((_, existing)) => *existing = value,
            None => properties.push((key.clone(), value)),
        }
        self.0
            .read_only_properties
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key);
    }

    /// Updates a non-replaceable read-only host property without making it
    /// writable to scripts. Returns `false` when `key` is not a read-only
    /// property or when it has been replaced by script-visible state.
    pub fn update_read_only(&self, key: &str, value: Value) -> bool {
        if self
            .0
            .read_only_properties
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(key)
        {
            let mut properties = self.properties();
            if let Some((_, current)) = properties.iter_mut().find(|(name, _)| name == key) {
                *current = value;
                return true;
            }
        }
        false
    }

    /// Defines a read-only host attribute that scripts may replace by
    /// assigning an own property with the same name.
    ///
    /// This models Web IDL's `[Replaceable] readonly` attributes. Subsequent
    /// calls to [`Object::set`] replace the host value with an ordinary
    /// writable property, after which [`Object::update_replaceable_read_only`]
    /// leaves the script's replacement intact.
    pub fn set_replaceable_read_only(&self, key: impl Into<String>, value: Value) {
        let key = key.into();
        self.set_read_only(key.clone(), value);
        self.0
            .replaceable_properties
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key);
    }

    /// Updates a host value previously defined with
    /// [`Object::set_replaceable_read_only`], unless script assignment has
    /// replaced that attribute. Returns `false` when an own replacement is
    /// present.
    pub fn update_replaceable_read_only(&self, key: &str, value: Value) -> bool {
        if !self
            .0
            .replaceable_properties
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(key)
        {
            return false;
        }
        let mut properties = self.properties();
        if let Some((_, current)) = properties.iter_mut().find(|(name, _)| name == key) {
            *current = value;
            true
        } else {
            false
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

    /// Deletes the own property `key`, if any.
    pub(crate) fn remove(&self, key: &str) {
        self.properties().retain(|(name, _)| name != key);
    }

    /// Returns whether the object itself, not its prototype chain, has a
    /// property named `key`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.properties().iter().any(|(name, _)| name == key)
            || self
                .0
                .accessors
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .any(|(name, _, _)| name == key)
    }

    /// Returns the own property names in insertion order.
    pub fn keys(&self) -> Vec<String> {
        let mut keys: Vec<_> = self
            .properties()
            .iter()
            .map(|(name, _)| name.clone())
            .collect();
        for (name, _, _) in self
            .0
            .accessors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            if !keys.contains(name) {
                keys.push(name.clone());
            }
        }
        if let Some(property_keys) = self
            .0
            .property_keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            && let Ok(dynamic_keys) = property_keys()
        {
            for name in dynamic_keys {
                if !keys.contains(&name) {
                    keys.push(name);
                }
            }
        }
        keys
    }

    /// Returns the number of own properties.
    pub fn len(&self) -> usize {
        self.keys().len()
    }

    /// Returns whether the object has no own properties.
    pub fn is_empty(&self) -> bool {
        self.keys().is_empty()
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
