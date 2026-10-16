//! Path authorization and sandbox-boundary checks.

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::FileAccessError;

/// A filesystem path that has passed the configured access policy.
///
/// The path is canonicalized before construction and can only be created by
/// [`FileAccessPolicy`]. Callers should retain this value instead of
/// re-authorizing or reconstructing the path from an untrusted URL.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizedPath(PathBuf);

impl AuthorizedPath {
    /// Returns the canonical path approved by the policy.
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// Default-deny policy for browser-authorized local files.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileAccessPolicy {
    allowed_roots: Vec<PathBuf>,
}

impl FileAccessPolicy {
    /// Creates a policy allowing reads below `allowed_roots`.
    ///
    /// Roots are canonicalized when the policy is created. An empty root list
    /// denies every request.
    pub fn new(allowed_roots: impl IntoIterator<Item = PathBuf>) -> Result<Self, FileAccessError> {
        let allowed_roots = allowed_roots
            .into_iter()
            .map(|root| fs::canonicalize(root).map_err(FileAccessError::from))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { allowed_roots })
    }

    /// Authorizes and canonicalizes one requested path.
    pub fn authorize_path(
        &self,
        requested: impl AsRef<Path>,
    ) -> Result<AuthorizedPath, FileAccessError> {
        let canonical = fs::canonicalize(requested).map_err(FileAccessError::from)?;
        if self
            .allowed_roots
            .iter()
            .any(|root| canonical.starts_with(root))
        {
            Ok(AuthorizedPath(canonical))
        } else {
            Err(FileAccessError::Denied)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_policy_denies_existing_paths() {
        let policy = FileAccessPolicy::default();
        assert_eq!(
            policy.authorize_path(std::env::temp_dir()),
            Err(FileAccessError::Denied)
        );
    }
}
