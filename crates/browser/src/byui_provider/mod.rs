//! Browser-owned `byui:` page provider.
//!
//! The provider implementation and the compile-time page registry are kept in
//! separate modules so adding a page does not require editing request
//! resolution logic. See `src/byui_provider/byui-provider.md` for the
//! page-authoring guide.

mod provider;
mod registry;

pub use provider::{BrowserPageProvider, provider};
