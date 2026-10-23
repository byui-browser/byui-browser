use net::{ProviderErrorKind, Request, RequestError};
use security::file_access::{FileAccessError, FileOrigin, FileRequestContext, FileRequestKind};

pub(super) fn provider_io_error() -> RequestError {
    RequestError::Provider {
        provider: "file",
        kind: ProviderErrorKind::Io,
    }
}

pub(super) fn provider_resource_exhausted_error() -> RequestError {
    RequestError::Provider {
        provider: "file",
        kind: ProviderErrorKind::ResourceExhausted,
    }
}

/// Translates networking request state into the security crate's file-access context.
pub(super) fn request_context(request: &Request) -> FileRequestContext {
    let initiating_origin = match request.environment().origin.as_ref() {
        None => FileOrigin::Absent,
        Some(origin) if origin.is_opaque() => FileOrigin::Opaque,
        Some(origin) => FileOrigin::Tuple(origin.as_str()),
    };
    let kind = if request.mode() == net::RequestMode::Navigate {
        FileRequestKind::Navigation
    } else {
        FileRequestKind::Subresource
    };
    FileRequestContext {
        initiating_origin,
        requested_origin: FileOrigin::LocalFile,
        kind,
    }
}

pub(super) fn provider_error(error: FileAccessError) -> RequestError {
    let kind = match error {
        FileAccessError::Missing => ProviderErrorKind::ResourceMissing,
        FileAccessError::Denied => ProviderErrorKind::AccessDenied,
        FileAccessError::InvalidPath => ProviderErrorKind::MalformedMetadata,
        FileAccessError::UnsupportedFile => ProviderErrorKind::UnsupportedResource,
        FileAccessError::Io => ProviderErrorKind::Io,
    };
    RequestError::Provider {
        provider: "file",
        kind,
    }
}
