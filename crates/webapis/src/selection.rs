//! The script-visible `Selection` object.

use std::sync::Arc;

use js::{JsError, JsResult, Object, Value};

/// Creates the initially-empty selection associated with a document.
///
/// Range and user-input integration are intentionally not implemented yet.
/// The exposed state follows the Selection API's initial empty selection, and
/// the object remains stable for the lifetime of its document.
pub(crate) fn new_selection() -> Object {
    let selection = Object::new();
    selection.set_read_only("anchorNode", Value::Null);
    selection.set_read_only("anchorOffset", Value::Number(0.0));
    selection.set_read_only("focusNode", Value::Null);
    selection.set_read_only("focusOffset", Value::Number(0.0));
    selection.set_read_only("isCollapsed", Value::Boolean(true));
    selection.set_read_only("rangeCount", Value::Number(0.0));
    selection.set_read_only("type", Value::String("None".into()));
    selection.set_read_only("direction", Value::String("none".into()));
    selection.set_method(
        "getRangeAt",
        Arc::new(|_| Err(JsError::new("IndexSizeError: the selection has no range"))),
    );
    selection.set_method("removeAllRanges", Arc::new(|_| Ok(Value::Undefined)));
    selection.set_method("empty", Arc::new(|_| Ok(Value::Undefined)));
    // The remaining methods have their empty-selection behavior. A non-empty
    // range cannot be created until this engine exposes Range and DOM boundary
    // points, so methods requiring one either no-op or report the exception
    // required for an empty selection.
    selection.set_method("addRange", Arc::new(|_| Ok(Value::Undefined)));
    selection.set_method(
        "removeRange",
        Arc::new(|_| Err(JsError::new("NotFoundError: range is not selected"))),
    );
    selection.set_method(
        "getComposedRanges",
        Arc::new(|_| {
            let ranges = Object::new();
            ranges.set_read_only("length", Value::Number(0.0));
            Ok(Value::Array(ranges))
        }),
    );
    selection.set_method("collapse", Arc::new(|_| Ok(Value::Undefined)));
    selection.set_method("setPosition", Arc::new(|_| Ok(Value::Undefined)));
    selection.set_method(
        "collapseToStart",
        Arc::new(|_| Err(JsError::new("InvalidStateError: selection is empty"))),
    );
    selection.set_method(
        "collapseToEnd",
        Arc::new(|_| Err(JsError::new("InvalidStateError: selection is empty"))),
    );
    selection.set_method(
        "extend",
        Arc::new(|_| Err(JsError::new("InvalidStateError: selection is empty"))),
    );
    selection.set_method("setBaseAndExtent", Arc::new(|_| Ok(Value::Undefined)));
    selection.set_method("selectAllChildren", Arc::new(|_| Ok(Value::Undefined)));
    selection.set_method("modify", Arc::new(|_| Ok(Value::Undefined)));
    selection.set_method("deleteFromDocument", Arc::new(|_| Ok(Value::Undefined)));
    selection.set_method("containsNode", Arc::new(|_| Ok(Value::Boolean(false))));
    selection.set_method("toString", Arc::new(|_| Ok(Value::String(String::new()))));
    selection
}

pub(crate) fn document_get_selection(selection: &Object, _: &[Value]) -> JsResult<Value> {
    Ok(Value::Object(selection.clone()))
}
