//! Browser adapter from security-owned file access to `net::FileProvider`.

mod admission;
mod authorization;
mod policy;
mod provider;
mod reader;
mod session;
mod stream;

pub(crate) use authorization::FileNavigationAuthorizations;
pub use authorization::FileNavigationError;
pub(crate) use provider::provider;
pub use security::file_access::FileAccessGrant;
pub use session::FileAccessGrantSource;

#[cfg(test)]
mod tests;
