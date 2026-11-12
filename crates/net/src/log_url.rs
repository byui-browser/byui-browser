//! Redaction helpers for log lines that mention URLs or request errors.
//!
//! URLs can carry `user:pass@` credentials, and `data:` URLs can be megabytes
//! long, so no log line prints a raw URL. Use [`loggable`] for every URL and
//! [`loggable_error`] for request errors that may embed one.

use std::borrow::Cow;

use crate::RequestError;

/// Longest URL, in characters, that is logged without truncation.
const MAX_LOGGED_URL_CHARS: usize = 300;

/// Returns a log-safe form of `url`.
///
/// `data:` URLs become `data:<N bytes>`, userinfo is stripped from
/// `scheme://user:pass@host`, and anything longer than 300 characters is
/// truncated with a `…(+N bytes)` suffix counting the dropped bytes.
pub(crate) fn loggable(url: &str) -> Cow<'_, str> {
    let trimmed = url.trim_start();
    if trimmed
        .get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:"))
    {
        return Cow::Owned(format!("data:<{} bytes>", url.len()));
    }
    let stripped = strip_userinfo(url);
    match truncate(&stripped) {
        Some(truncated) => Cow::Owned(truncated),
        None => stripped,
    }
}

/// Returns a log-safe summary of `error`.
///
/// `InvalidUrl` embeds the raw URL and `Transport` embeds the transport's
/// message, which includes the URL, so both are reduced to a short kind.
/// Every other variant prints its normal `Display`.
pub(crate) fn loggable_error(error: &RequestError) -> Cow<'static, str> {
    match error {
        RequestError::InvalidUrl(_) => Cow::Borrowed("invalid url"),
        RequestError::Transport(_) => Cow::Borrowed("transport failure"),
        other => Cow::Owned(other.to_string()),
    }
}

fn strip_userinfo(url: &str) -> Cow<'_, str> {
    if let Ok(mut parsed) = url::Url::parse(url) {
        if parsed.username().is_empty() && parsed.password().is_none() {
            return Cow::Borrowed(url);
        }
        // Both setters only fail for cannot-be-a-base URLs, which have no
        // userinfo to begin with.
        let _ = parsed.set_username("");
        let _ = parsed.set_password(None);
        return Cow::Owned(parsed.to_string());
    }
    let Some(scheme_end) = url.find("://") else {
        return Cow::Borrowed(url);
    };
    let authority_start = scheme_end + 3;
    let rest = &url[authority_start..];
    let authority_end = rest.find('/').unwrap_or(rest.len());
    match rest[..authority_end].rfind('@') {
        Some(at) => Cow::Owned(format!("{}{}", &url[..authority_start], &rest[at + 1..])),
        None => Cow::Borrowed(url),
    }
}

fn truncate(text: &str) -> Option<String> {
    let (cut, _) = text.char_indices().nth(MAX_LOGGED_URL_CHARS)?;
    Some(format!("{}…(+{} bytes)", &text[..cut], text.len() - cut))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_urls_log_only_their_size() {
        let url = format!("data:text/plain;base64,{}", "A".repeat(1000));
        assert_eq!(loggable(&url), format!("data:<{} bytes>", url.len()));
    }

    #[test]
    fn userinfo_is_stripped() {
        assert_eq!(
            loggable("https://user:secret@example.com/a?b=1"),
            "https://example.com/a?b=1"
        );
        assert_eq!(
            loggable("https://example.com/a@b"),
            "https://example.com/a@b"
        );
    }

    #[test]
    fn userinfo_is_stripped_when_parsing_fails() {
        let logged = loggable("http://user:secret@exa mple.com:99999/path@x");
        assert!(!logged.contains("secret"), "{logged}");
        assert_eq!(logged, "http://exa mple.com:99999/path@x");
    }

    #[test]
    fn long_urls_are_truncated() {
        let url = format!("https://example.com/{}", "a".repeat(400));
        let logged = loggable(&url);
        assert!(logged.starts_with("https://example.com/aaa"));
        assert!(logged.ends_with(&format!("…(+{} bytes)", url.len() - 300)));
        assert_eq!(loggable("https://example.com/"), "https://example.com/");
    }

    #[test]
    fn request_errors_do_not_leak_urls() {
        let error = RequestError::InvalidUrl("http://u:p@x".into());
        assert_eq!(loggable_error(&error), "invalid url");
        assert!(!loggable_error(&RequestError::Transport("http://u:p@x".into())).contains("u:p"));
        assert_eq!(
            loggable_error(&RequestError::CorsDenied),
            "CORS check denied the response"
        );
    }
}
