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

/// Accepts only formats safe for the current navigation-only static mode.
/// Script, SVG, JSON, and untyped files remain denied until a browser-owned
/// document/container policy can enforce their consumers end to end.
pub(super) fn allows_content(
    policy: crate::FileAccessContentPolicy,
    media_type: Option<&str>,
) -> bool {
    match policy {
        crate::FileAccessContentPolicy::StaticDocument => matches!(
            media_type,
            Some("text/html; charset=utf-8")
                | Some("text/css; charset=utf-8")
                | Some("image/png")
                | Some("image/jpeg")
                | Some("image/gif")
                | Some("image/webp")
        ),
    }
}

/// Returns whether a bounded prefix identifies a native executable on this
/// platform. This is intentionally a conservative format check, not malware
/// detection or a claim to recognize every executable/polyglot format.
pub(super) fn is_native_executable_prefix(prefix: &[u8]) -> bool {
    #[cfg(windows)]
    {
        prefix.starts_with(b"MZ")
    }
    #[cfg(target_os = "macos")]
    {
        matches!(
            prefix,
            [0xfe, 0xed, 0xfa, 0xce, ..]
                | [0xce, 0xfa, 0xed, 0xfe, ..]
                | [0xfe, 0xed, 0xfa, 0xcf, ..]
                | [0xcf, 0xfa, 0xed, 0xfe, ..]
                | [0xca, 0xfe, 0xba, 0xbe, ..]
        )
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        prefix.starts_with(b"\x7fELF")
    }
}
