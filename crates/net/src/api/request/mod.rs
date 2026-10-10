//! Fetch-style request state and transport preparation.

mod body;
mod context;
mod definition;
mod origin;
mod prepared;

pub use body::RequestBody;
pub use context::{
    CacheMode, CredentialsMode, FetchContext, FetchEnvironment, InitiatorType, RedirectMode,
    Referrer, ReferrerPolicy, RequestDestination, RequestMode, ServiceWorkersMode,
};
pub use definition::{Request, RequestPriority};
pub use origin::{NetworkPartitionKey, Origin};
pub(crate) use prepared::PreparedRequest;
