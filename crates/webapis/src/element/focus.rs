//! `HTMLElement.focus()` on script element wrappers.
//!
//! A connected element is focusable when it is one of `button`, `select`,
//! `textarea`, `summary`, or `iframe`; an `input` whose type is not `hidden`;
//! an `a` or `area` with `href`; an element with a `tabindex` attribute; or
//! an editing host (`contenteditable` of `""` or `"true"`). `disabled` and
//! `hidden` attributes, and a computed `display` of `none`, keep it from
//! taking focus. Anything else, including a disconnected element, is ignored.
//!
//! A successful call records the element and dispatches `blur` on the
//! previously focused element (when that element is still connected) and
//! `focus` on this one. Both events do not bubble. Focusing the element that
//! is already focused does not dispatch again.
//!
//! Limitations: `preventScroll` is accepted and ignored, because this engine
//! does not scroll the page. `focusin` and `focusout` are not dispatched.
//! `relatedTarget` is not set. A disabled fieldset does not disable its
//! descendants. Removing the focused element does not dispatch `blur`.

use std::sync::{Arc, Mutex};

use html::NodeID;
use js::{JsResult, Object, Value};

use crate::document::Document;
use crate::element::Element;
use crate::event::{dispatch_event_at, event_object};

use super::wrap_element;

/// Installs `HTMLElement.focus()` on the wrapper `object` for `element`.
pub(super) fn install(object: &Object, element: &Element, document: &Arc<Mutex<Document>>) {
    let node = element.node;
    let document = Arc::clone(document);
    object.set_method(
        "focus",
        Arc::new(move |arguments| {
            if let Some(Value::Object(options)) = arguments.first()
                && options.get("preventScroll") == Some(Value::Boolean(true))
            {
                log::debug!(
                    "Element.focus() ignores preventScroll; the element is not scrolled into view"
                );
            }
            let (focusable, previous) = {
                let locked = document.lock().unwrap();
                (is_focusable(&locked, node), locked.focused_element())
            };
            if !focusable {
                log::debug!("Element.focus() ignored a non-focusable element");
                return Ok(Value::Undefined);
            }
            if previous == Some(node) {
                return Ok(Value::Undefined);
            }
            let previous_connected =
                previous.is_some_and(|previous| document.lock().unwrap().is_connected(previous));
            document.lock().unwrap().set_focused_element(Some(node));
            if let Some(previous) = previous.filter(|_| previous_connected) {
                dispatch_simple(&document, previous, "blur")?;
            }
            dispatch_simple(&document, node, "focus")?;
            Ok(Value::Undefined)
        }),
    );
}

/// Reports whether `node` can take script focus.
fn is_focusable(document: &Document, node: NodeID) -> bool {
    if !document.is_connected(node) {
        return false;
    }
    let Some(element) = document.element_snapshot(node) else {
        return false;
    };
    if element.get_attribute("disabled").is_some() || element.get_attribute("hidden").is_some() {
        return false;
    }
    if document.computed_styles().value_for(node, "display") == Some("none") {
        return false;
    }
    let editable = matches!(
        element
            .get_attribute("contenteditable")
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("" | "true")
    );
    if element.get_attribute("tabindex").is_some() || editable {
        return true;
    }
    match element.local_name.as_str() {
        "button" | "select" | "textarea" | "summary" | "iframe" => true,
        "input" => element
            .get_attribute("type")
            .is_none_or(|value| !value.eq_ignore_ascii_case("hidden")),
        "a" | "area" => element.get_attribute("href").is_some(),
        _ => false,
    }
}

/// Dispatches a non-bubbling event at `node`'s element wrapper.
fn dispatch_simple(
    document: &Arc<Mutex<Document>>,
    node: NodeID,
    event_type: &str,
) -> JsResult<()> {
    let Value::Object(target) = wrap_element(document, node)? else {
        return Ok(());
    };
    let event = event_object(event_type.to_owned(), false, false);
    let snapshot = document.lock().unwrap().clone();
    dispatch_event_at(&snapshot, Some(node), &target, &[Value::Object(event)])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use html::parse_raw_html;
    use js::{Realm, Value};

    use crate::document::{Document, register_document};

    fn realm_with(markup: &str) -> Realm {
        let mut realm = Realm::new();
        register_document(
            &mut realm,
            Document::from_html_document(&parse_raw_html(markup.to_owned())),
        )
        .unwrap();
        realm
    }

    fn eval(realm: &mut Realm, script: &str) -> Value {
        realm
            .evaluate_script(script)
            .unwrap_or_else(|error| panic!("{script}: {error:?}"))
    }

    fn string(value: &str) -> Value {
        Value::String(value.to_owned())
    }

    #[test]
    fn focus_moves_between_focusable_elements_and_fires_blur() {
        let mut realm = realm_with(
            "<button id='one'></button><button id='two'></button><div id='plain'></div>\
             <div id='tab' tabindex='-1'></div><a id='link' href='/x'></a>\
             <input id='hidden' type='hidden'><button id='off' disabled></button>\
             <button id='gone' style='display: none'></button>",
        );
        assert_eq!(
            eval(
                &mut realm,
                "var log = ''; \
                 var one = document.getElementById('one'); \
                 var two = document.getElementById('two'); \
                 one.addEventListener('focus', () => { log = log + 'one-focus;'; }); \
                 one.addEventListener('blur', () => { log = log + 'one-blur;'; }); \
                 two.addEventListener('focus', () => { log = log + 'two-focus;'; }); \
                 document.getElementById('plain').addEventListener('focus', () => { log = log + 'plain;'; }); \
                 document.getElementById('hidden').addEventListener('focus', () => { log = log + 'hidden;'; }); \
                 document.getElementById('off').addEventListener('focus', () => { log = log + 'off;'; }); \
                 document.getElementById('gone').addEventListener('focus', () => { log = log + 'gone;'; }); \
                 one.focus(); one.focus(); \
                 document.getElementById('plain').focus(); \
                 document.getElementById('hidden').focus(); \
                 document.getElementById('off').focus(); \
                 document.getElementById('gone').focus(); \
                 document.createElement('button').focus(); \
                 two.focus({preventScroll: true}); \
                 log"
            ),
            string("one-focus;one-blur;two-focus;")
        );
        assert_eq!(
            eval(
                &mut realm,
                "var more = ''; \
                 document.getElementById('tab').addEventListener('focus', () => { more = more + 'tab;'; }); \
                 document.getElementById('link').addEventListener('focus', () => { more = more + 'link;'; }); \
                 document.getElementById('tab').focus(); \
                 document.getElementById('link').focus(); \
                 more"
            ),
            string("tab;link;")
        );
    }

    #[test]
    fn focus_accepts_contenteditable_and_does_not_bubble() {
        let mut realm = realm_with("<div id='host'><div id='edit' contenteditable></div></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var log = ''; \
                 document.getElementById('host').addEventListener('focus', () => { log = log + 'bubble;'; }); \
                 document.getElementById('edit').addEventListener('focus', () => { log = log + 'edit;'; }); \
                 document.getElementById('edit').focus(); log"
            ),
            string("edit;")
        );
    }
}
