//! Arrays: dense, index-addressed element lists shared by reference.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::conversions::to_number;
use super::{Function, Object};
use crate::{HostFunction, JsError, JsResult, Value};

/// The largest length an [`Array`] can have.
///
/// Arrays store every element up to their length, so the limit keeps a script
/// such as `a[4000000000] = 1` or `a.length = 1e9` from allocating gigabytes.
/// Exceeding it is a runtime error rather than a sparse array.
pub const MAX_ARRAY_LENGTH: usize = 1 << 20;

/// The first number that is not an array index; `2**32 - 1` and above are
/// ordinary property names, as in JavaScript.
const INDEX_LIMIT: f64 = 4_294_967_295.0;

/// A JavaScript array value: an ordered list of elements that may contain
/// holes.
///
/// `Array` is a handle. Cloning it is cheap and yields another reference to
/// the same elements, and clones compare equal with `==` and `===`, while
/// separately created arrays never do, even with the same contents.
///
/// An element is either a value or a *hole*, a position that was never
/// assigned (`[1, , 3]`, or the gap left by `a[5] = x` past the end). A hole
/// reads as `undefined`, like an assigned `undefined`, but [`Array::has`] and
/// [`Array::get`] tell them apart. `length` is always one more than the
/// highest index in use unless it was set larger.
///
/// Scripts address elements by canonical index keys: the number `1` and the
/// string `'1'` name the same element, while `'01'`, `'1.5'`, and `-1` are
/// ordinary named properties stored beside the elements and not counted in
/// `length`. Reading `length` gives the element count; assigning it
/// truncates or pads with holes, and a value that is not a whole number from
/// `0` to [`MAX_ARRAY_LENGTH`] is an error. Reading `push` or `pop` yields a
/// function bound to this array; a named property of the same name takes
/// precedence.
///
/// Limitations: there is no `Array.prototype`, `Array` global, or iteration
/// support. Arrays are dense, so lengths above [`MAX_ARRAY_LENGTH`] are
/// rejected. A bound `push` or `pop` is created on every read, so
/// `a.push === a.push` is false. As with objects there is no garbage
/// collector, so an array that contains itself is never freed.
#[derive(Clone, Default)]
pub struct Array(Arc<ArrayData>);

#[derive(Default)]
struct ArrayData {
    elements: Mutex<Vec<Option<Value>>>,
    properties: Object,
}

impl Array {
    /// Creates an empty array with a new identity.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an array holding `values` in order, with no holes.
    ///
    /// Fails if there are more than [`MAX_ARRAY_LENGTH`] values.
    pub fn from_values(values: impl IntoIterator<Item = Value>) -> JsResult<Self> {
        let elements: Vec<_> = values.into_iter().map(Some).collect();
        if elements.len() > MAX_ARRAY_LENGTH {
            return Err(length_error());
        }
        let array = Self::new();
        *array.elements() = elements;
        Ok(array)
    }

    fn elements(&self) -> MutexGuard<'_, Vec<Option<Value>>> {
        // Nothing panics while holding the lock, so the list is consistent.
        self.0
            .elements
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Returns the array's `length`.
    pub fn len(&self) -> usize {
        self.elements().len()
    }

    /// Returns whether the array's `length` is zero.
    pub fn is_empty(&self) -> bool {
        self.elements().is_empty()
    }

    /// Returns the value at `index`, or `None` for a hole or an index past
    /// the end.
    pub fn get(&self, index: usize) -> Option<Value> {
        self.elements().get(index).cloned().flatten()
    }

    /// Returns whether `index` holds a value, as opposed to being a hole or
    /// past the end.
    pub fn has(&self, index: usize) -> bool {
        self.elements().get(index).is_some_and(Option::is_some)
    }

    /// Stores `value` at `index`, padding any gap before it with holes and
    /// growing `length` when `index` is at or past the end.
    ///
    /// Fails, changing nothing, if that would exceed [`MAX_ARRAY_LENGTH`].
    pub fn set(&self, index: usize, value: Value) -> JsResult<()> {
        let mut elements = self.elements();
        if index >= elements.len() {
            if index >= MAX_ARRAY_LENGTH {
                return Err(length_error());
            }
            elements.resize(index + 1, None);
        }
        elements[index] = Some(value);
        Ok(())
    }

    /// Sets `length`, dropping elements past it or padding with holes.
    ///
    /// Fails, changing nothing, if `length` exceeds [`MAX_ARRAY_LENGTH`].
    pub fn set_len(&self, length: usize) -> JsResult<()> {
        if length > MAX_ARRAY_LENGTH {
            return Err(length_error());
        }
        self.elements().resize(length, None);
        Ok(())
    }

    /// Appends `value`, returning the new length.
    ///
    /// Fails if the array is already [`MAX_ARRAY_LENGTH`] long.
    pub fn push(&self, value: Value) -> JsResult<usize> {
        self.append(Some(value))
    }

    /// Appends a hole, returning the new length.
    ///
    /// Fails if the array is already [`MAX_ARRAY_LENGTH`] long.
    pub fn push_hole(&self) -> JsResult<usize> {
        self.append(None)
    }

    fn append(&self, element: Option<Value>) -> JsResult<usize> {
        let mut elements = self.elements();
        if elements.len() >= MAX_ARRAY_LENGTH {
            return Err(length_error());
        }
        elements.push(element);
        Ok(elements.len())
    }

    /// Removes and returns the last element. A hole reads as `undefined`, and
    /// an empty array yields `None`.
    pub fn pop(&self) -> Option<Value> {
        let element = self.elements().pop()?;
        Some(element.unwrap_or(Value::Undefined))
    }

    /// Returns every element in order, with holes as `undefined`.
    pub fn to_vec(&self) -> Vec<Value> {
        self.elements()
            .iter()
            .map(|element| element.clone().unwrap_or(Value::Undefined))
            .collect()
    }

    /// Reads the property a script names with `key`: `length`, an element, a
    /// bound `push` or `pop`, or a named property. Anything missing is
    /// `undefined`.
    pub fn get_property(&self, key: &str) -> Value {
        if key == "length" {
            return Value::Number(self.len() as f64);
        }
        if let Some(index) = array_index(key) {
            return self.get(index).unwrap_or(Value::Undefined);
        }
        if let Some(value) = self.0.properties.get_own(key) {
            return value;
        }
        match key {
            "push" => self.bound_method("push", |array, arguments| {
                let mut length = array.len();
                for argument in arguments {
                    length = array.push(argument.clone())?;
                }
                Ok(Value::Number(length as f64))
            }),
            "pop" => self.bound_method("pop", |array, _| {
                Ok(array.pop().unwrap_or(Value::Undefined))
            }),
            _ => Value::Undefined,
        }
    }

    /// Writes the property a script names with `key`, as `array[key] = value`
    /// does. See [`Array`] for how keys map to elements, `length`, and named
    /// properties.
    pub fn set_property(&self, key: &str, value: Value) -> JsResult<()> {
        if key == "length" {
            let number = to_number(&value);
            if number.fract() != 0.0 || !(0.0..=MAX_ARRAY_LENGTH as f64).contains(&number) {
                return Err(length_error());
            }
            return self.set_len(number as usize);
        }
        match array_index(key) {
            Some(index) => self.set(index, value),
            None => {
                self.0.properties.set(key, value);
                Ok(())
            }
        }
    }

    fn bound_method(
        &self,
        name: &str,
        body: impl Fn(&Array, &[Value]) -> JsResult<Value> + Send + Sync + 'static,
    ) -> Value {
        let array = self.clone();
        let function: HostFunction = Arc::new(move |arguments| body(&array, arguments));
        Value::Function(Function::from_host(name, function))
    }

    fn join(&self, visiting: &mut Vec<*const ArrayData>) -> String {
        let identity = Arc::as_ptr(&self.0);
        if visiting.contains(&identity) {
            return String::new();
        }
        visiting.push(identity);
        let elements: Vec<_> = self.elements().clone();
        let parts: Vec<String> = elements
            .iter()
            .map(|element| match element {
                None | Some(Value::Undefined | Value::Null) => String::new(),
                Some(Value::Array(inner)) => inner.join(visiting),
                Some(other) => other.to_string(),
            })
            .collect();
        visiting.pop();
        parts.join(",")
    }
}

/// Returns the element position `key` names, if it is a canonical array
/// index: decimal digits without a leading zero, below `2**32 - 1`.
pub(crate) fn array_index(key: &str) -> Option<usize> {
    let digits = key.as_bytes();
    if digits.is_empty()
        || digits.len() > 10
        || !digits.iter().all(u8::is_ascii_digit)
        || (digits.len() > 1 && digits[0] == b'0')
    {
        return None;
    }
    let index: f64 = key.parse().ok()?;
    (index < INDEX_LIMIT).then_some(index as usize)
}

/// Returns the element position a number key names, if it is an array index,
/// so callers can skip building the key string.
pub(crate) fn number_index(number: f64) -> Option<usize> {
    ((0.0..INDEX_LIMIT).contains(&number) && number.fract() == 0.0).then_some(number as usize)
}

fn length_error() -> JsError {
    JsError::with_context(
        crate::JsErrorCategory::Runtime,
        format!("Invalid array length: arrays are limited to {MAX_ARRAY_LENGTH} elements"),
        "array length",
    )
}

impl PartialEq for Array {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Lists only the length, so arrays that contain themselves do not recurse
/// forever.
impl fmt::Debug for Array {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Array")
            .field("length", &self.len())
            .finish()
    }
}

/// The JavaScript string form of an array: its elements joined by commas,
/// with holes, `undefined`, and `null` as empty text. An array met again
/// while printing itself prints as empty text.
impl fmt::Display for Array {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.join(&mut Vec::new()))
    }
}
