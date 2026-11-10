//! Fetch-style request state and transport preparation.

mod accessors;
mod constructors;
mod context;
mod derived;
mod mutators;
mod origin;
mod prepared;
mod redirect;
mod state;

pub use context::{
    CacheMode, CredentialsMode, FetchContext, FetchEnvironment, InitiatorType, RedirectMode,
    Referrer, ReferrerPolicy, RequestDestination, RequestMode, ServiceWorkersMode,
};
pub use origin::{NetworkPartitionKey, Origin};
pub(crate) use prepared::PreparedRequest;
pub use state::{Request, RequestDuplex, RequestPriority};

#[cfg(test)]
mod tests;
