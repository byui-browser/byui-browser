//! In-memory permission decisions for web origins.
//!
//! This is the first, intentionally small permission layer. Decisions last
//! only as long as the policy exists; persistence, user prompts, and IPC will
//! be added later when the surrounding browser infrastructure is ready.

use std::collections::HashMap;

use crate::Origin;

/// A protected browser capability that a website may request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionKind {
    /// Access to origin-scoped client-side storage.
    Storage,
    /// Access to HTTP cookies associated with the origin.
    Cookies,
    /// Access to the user's location.
    Geolocation,
}

/// The decision recorded for a permission request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionState {
    /// The origin may use the requested capability.
    Granted,
    /// The origin may not use the requested capability.
    Denied,
}

/// An in-memory permission policy keyed by origin and capability.
///
/// A missing decision is treated as [`PermissionState::Denied`]. This
/// fail-closed default prevents a website from receiving access merely
/// because nobody has made a decision about it yet.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PermissionPolicy {
    decisions: HashMap<(Origin, PermissionKind), PermissionState>,
}

impl PermissionPolicy {
    /// Creates an empty policy with no granted permissions.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the decision for `origin` and `permission`.
    ///
    /// Unrecorded permissions are denied.
    #[must_use]
    pub fn check(&self, origin: &Origin, permission: PermissionKind) -> PermissionState {
        self.decisions
            .get(&(origin.clone(), permission))
            .copied()
            .unwrap_or(PermissionState::Denied)
    }

    /// Grants `permission` to `origin` for the lifetime of this policy.
    pub fn grant(&mut self, origin: Origin, permission: PermissionKind) {
        self.decisions
            .insert((origin, permission), PermissionState::Granted);
    }

    /// Explicitly denies `permission` for `origin`.
    pub fn deny(&mut self, origin: Origin, permission: PermissionKind) {
        self.decisions
            .insert((origin, permission), PermissionState::Denied);
    }

    /// Removes the recorded decision for `origin` and `permission`.
    ///
    /// After revocation, [`Self::check`] returns [`PermissionState::Denied`]
    /// because denial is the default state.
    pub fn revoke(&mut self, origin: &Origin, permission: PermissionKind) {
        self.decisions.remove(&(origin.clone(), permission));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(scheme: &str, host: &str, port: u16) -> Origin {
        Origin::with_port(scheme, host, port)
    }

    #[test]
    fn new_policy_denies_unrecorded_permissions() {
        let policy = PermissionPolicy::new();
        let site = origin("https", "example.com", 443);

        assert_eq!(
            policy.check(&site, PermissionKind::Storage),
            PermissionState::Denied
        );
    }

    #[test]
    fn grant_allows_only_the_selected_origin_and_permission() {
        let site = origin("https", "example.com", 443);
        let other_site = origin("https", "other.example", 443);
        let mut policy = PermissionPolicy::new();

        policy.grant(site.clone(), PermissionKind::Storage);

        assert_eq!(
            policy.check(&site, PermissionKind::Storage),
            PermissionState::Granted
        );
        assert_eq!(
            policy.check(&other_site, PermissionKind::Storage),
            PermissionState::Denied
        );
        assert_eq!(
            policy.check(&site, PermissionKind::Cookies),
            PermissionState::Denied
        );
    }

    #[test]
    fn deny_overrides_a_previous_grant() {
        let site = origin("https", "example.com", 443);
        let mut policy = PermissionPolicy::new();

        policy.grant(site.clone(), PermissionKind::Geolocation);
        policy.deny(site.clone(), PermissionKind::Geolocation);

        assert_eq!(
            policy.check(&site, PermissionKind::Geolocation),
            PermissionState::Denied
        );
    }

    #[test]
    fn revoke_returns_permission_to_default_denial() {
        let site = origin("https", "example.com", 443);
        let mut policy = PermissionPolicy::new();

        policy.grant(site.clone(), PermissionKind::Cookies);
        policy.revoke(&site, PermissionKind::Cookies);

        assert_eq!(
            policy.check(&site, PermissionKind::Cookies),
            PermissionState::Denied
        );
    }
}
