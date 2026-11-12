//! Bindings between JavaScript and DOM/platform APIs.
//!
//! **Owning team**: JS APIs (Web APIs) Team
//!
//! Window, Document, Element, Fetch, timers, events, and related APIs.
//! DOM mutations from JS must go through a controlled mutation interface
//! so style/layout invalidation stays correct.

#![forbid(unsafe_code)]

mod alert;
mod console;
mod custom_elements;
mod document;
mod dom_exception;
mod element;
mod event;
mod fetch;
pub mod local_storage;
mod location;
mod selector;
pub mod session_storage;
mod timer;
mod window;

pub use alert::{AlertSink, alert, register_alert};
pub use console::{ConsoleSink, print, register_console, register_print};
pub use custom_elements::CustomElementRegistry;
pub use document::{
    Document, get_element_by_id, register_document, register_document_on_window,
    register_document_with_location,
};
pub use dom_exception::DomException;
pub use element::{Element, get_attribute};
pub use event::{
    abort_controller_constructor, add_event_listener, dispatch_click, dispatch_click_at,
    dispatch_event, event_constructor, remove_event_listener,
};
pub use fetch::{fetch, register_fetch, register_fetch_with_url};
pub use location::{
    LocationNavigate, NavigationBehavior, register_location, register_location_with_navigation,
};
pub use selector::SelectorError;
pub use timer::{TimerId, TimerQueue};
pub use window::{
    ViewportSize, register_window, register_window_with_security_context,
    register_window_with_security_context_and_viewport, register_window_with_url,
    register_window_with_url_and_device_pixel_ratio,
    register_window_with_url_device_pixel_ratio_and_viewport, run_window_animation_frames,
    run_window_microtasks, run_window_timers,
};
