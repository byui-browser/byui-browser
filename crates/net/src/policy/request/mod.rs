//! Request policy validation and scheme dispatch.

mod redirect;
mod scheme;
mod validation;

pub(crate) use scheme::FetchScheme;
pub(crate) use validation::RequestPolicy;

#[cfg(test)]
mod tests;
