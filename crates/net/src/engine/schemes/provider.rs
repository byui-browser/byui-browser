//! Shared dispatch for browser-owned provider-backed schemes.

use reqwest::Url;

use crate::{
    api::{
        error::RequestError,
        providers::{ProviderFuture, SchemeProviders, SchemeResource},
        request::{PreparedRequest, Request},
        response::Response,
    },
    engine::RequestController,
    policy::FetchScheme,
};

/// Selects the browser-owned capability that resolves a local scheme.
#[derive(Clone, Copy)]
enum ProviderKind {
    /// Browser-owned object URLs.
    Blob,
    /// Browser-authorized filesystem resources.
    File,
    /// Browser-owned registered pages.
    Page,
}

/// Method policy applied before provider selection.
#[derive(Clone, Copy)]
enum MethodPolicy {
    /// Permit resource reads through `GET` and metadata reads through `HEAD`.
    ReadOnly,
}

/// Declarative metadata for one provider-backed local scheme.
#[derive(Clone, Copy)]
pub(super) struct ProviderScheme {
    fetch_scheme: FetchScheme,
    provider: ProviderKind,
    method_policy: MethodPolicy,
    method_feature: &'static str,
    provider_name: &'static str,
}

const BLOB_SCHEME: ProviderScheme = ProviderScheme {
    fetch_scheme: FetchScheme::Blob,
    provider: ProviderKind::Blob,
    method_policy: MethodPolicy::ReadOnly,
    method_feature: "blob scheme method",
    provider_name: "blob",
};

const FILE_SCHEME: ProviderScheme = ProviderScheme {
    fetch_scheme: FetchScheme::File,
    provider: ProviderKind::File,
    method_policy: MethodPolicy::ReadOnly,
    method_feature: "file scheme method",
    provider_name: "file",
};

const BYUI_SCHEME: ProviderScheme = ProviderScheme {
    fetch_scheme: FetchScheme::Byui,
    provider: ProviderKind::Page,
    method_policy: MethodPolicy::ReadOnly,
    method_feature: "byui scheme method",
    provider_name: "byui page",
};

impl ProviderScheme {
    /// Selects metadata for a scheme already classified as provider-backed.
    pub(super) const fn for_fetch_scheme(scheme: FetchScheme) -> Self {
        match scheme {
            FetchScheme::Blob => BLOB_SCHEME,
            FetchScheme::File => FILE_SCHEME,
            FetchScheme::Byui => BYUI_SCHEME,
            FetchScheme::Http | FetchScheme::Https | FetchScheme::Data | FetchScheme::About => {
                unreachable!()
            }
        }
    }

    /// Selects exactly one provider and starts its scheme-specific operation.
    fn resolve<'a>(
        self,
        providers: &'a SchemeProviders,
        request: &'a Request,
        url: &'a Url,
    ) -> Result<ProviderFuture<'a>, RequestError> {
        let missing_provider = || {
            log::debug!(
                "no {} provider installed for {}",
                self.provider_name,
                crate::log_url::loggable(url.as_str())
            );
            RequestError::MissingProvider(self.provider_name)
        };

        Ok(match self.provider {
            ProviderKind::Blob => providers
                .blob()
                .ok_or_else(missing_provider)?
                .resolve(request, url),
            ProviderKind::File => providers
                .file()
                .ok_or_else(missing_provider)?
                .open(request, url),
            ProviderKind::Page => providers
                .page()
                .ok_or_else(missing_provider)?
                .resolve(request, url),
        })
    }
}

/// Rejects methods that cannot retrieve an immutable provider-owned resource.
///
/// This runs before provider selection so unsupported methods cannot trigger
/// provider side effects.
fn require_read_method(request: &Request, scheme: ProviderScheme) -> Result<(), RequestError> {
    let permitted = match scheme.method_policy {
        MethodPolicy::ReadOnly => matches!(
            request.method(),
            &reqwest::Method::GET | &reqwest::Method::HEAD
        ),
    };
    if permitted {
        return Ok(());
    }

    let feature = scheme.method_feature;
    log::warn!(
        "rejected {} request to provider-backed scheme: {feature}",
        request.method()
    );
    Err(RequestError::UnsupportedFeature(feature))
}

/// Waits for a provider without allowing cancellation to leave its operation
/// or any resources held by its future running in the background.
async fn await_provider(
    request: &Request,
    operation: ProviderFuture<'_>,
) -> Result<SchemeResource, RequestError> {
    if request.signal().is_aborted() {
        log::debug!("provider wait skipped: request already aborted");
        return Err(RequestError::Aborted);
    }
    tokio::select! {
        _ = request.signal().cancelled() => Err(RequestError::Aborted),
        result = operation => result,
    }
}

impl RequestController {
    /// Executes one provider-backed scheme through the shared policy and response path.
    pub(super) async fn fetch_provider_scheme(
        &self,
        prepared: PreparedRequest,
        scheme: ProviderScheme,
    ) -> Result<Response, RequestError> {
        let (request, url) = prepared.into_parts();
        require_read_method(&request, scheme)?;
        let operation = scheme.resolve(&self.inner.providers, &request, &url)?;
        let resource = await_provider(&request, operation).await?;
        self.finalize_scheme_resource(&request, resource, scheme.fetch_scheme)
    }
}
