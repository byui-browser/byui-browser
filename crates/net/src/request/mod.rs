//! Fetch-style request state and transport preparation.

mod body;
mod context;
mod headers;
mod origin;
mod prepared;
mod request;

pub use body::RequestBody;
pub use context::{
    CacheMode, CredentialsMode, FetchContext, FetchEnvironment, InitiatorType, RedirectMode,
    Referrer, ReferrerPolicy, RequestDestination, RequestMode, ServiceWorkersMode,
};
pub use headers::{HeaderGuard, HeaderList};
pub use origin::{NetworkPartitionKey, Origin};
pub(crate) use prepared::PreparedRequest;
pub use request::{Request, RequestPriority};
