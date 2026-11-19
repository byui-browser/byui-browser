use std::collections::HashMap;

use http::header::{HeaderMap, HeaderName, HeaderValue};

use super::validation::{append_header_value_string, header_value_string, is_non_combinable};

/// Raw ordered header storage without Fetch mutation or exposure policy.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct HeaderStore {
    entries: Vec<(HeaderName, HeaderValue)>,
}

impl HeaderStore {
    /// Returns whether the store contains no fields.
    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Appends a raw field while preserving insertion order and duplicates.
    pub(super) fn append(&mut self, name: HeaderName, value: HeaderValue) {
        self.entries.push((name, value));
    }

    /// Replaces every value for `name` with one field at the end of the list.
    pub(super) fn replace(&mut self, name: HeaderName, value: HeaderValue) {
        self.delete(&name);
        self.append(name, value);
    }

    /// Deletes every field matching `name`.
    pub(super) fn delete(&mut self, name: &HeaderName) {
        self.entries.retain(|(existing, _)| existing != name);
    }

    /// Retains only fields accepted by `predicate`.
    pub(super) fn retain(&mut self, mut predicate: impl FnMut(&HeaderName, &HeaderValue) -> bool) {
        self.entries.retain(|(name, value)| predicate(name, value));
    }

    /// Returns whether an already parsed header name is present.
    pub(super) fn contains_name(&self, name: &HeaderName) -> bool {
        self.entries.iter().any(|(existing, _)| existing == name)
    }

    /// Iterates over the raw ordered fields.
    pub(super) fn iter(&self) -> impl Iterator<Item = (&HeaderName, &HeaderValue)> {
        self.entries.iter().map(|(name, value)| (name, value))
    }

    /// Returns the Fetch-combined string value for `name`.
    pub(super) fn get(&self, name: &str) -> Option<String> {
        let mut combined = None::<String>;
        for (entry_name, value) in &self.entries {
            if !entry_name.as_str().eq_ignore_ascii_case(name) {
                continue;
            }

            if name.eq_ignore_ascii_case("set-cookie") {
                return Some(header_value_string(value));
            }
            match &mut combined {
                Some(combined) => {
                    combined.push_str(", ");
                    append_header_value_string(combined, value);
                }
                None => combined = Some(header_value_string(value)),
            }
        }
        combined
    }

    /// Combines duplicate ordinary fields in one pass over the raw list.
    pub(super) fn combined_entries(&self) -> Vec<(HeaderName, String)> {
        let mut combined = Vec::<(HeaderName, String)>::new();
        let mut positions = HashMap::<HeaderName, usize>::new();

        for (name, value) in &self.entries {
            if is_non_combinable(name) {
                combined.push((name.clone(), header_value_string(value)));
                continue;
            }

            if let Some(position) = positions.get(name).copied() {
                let existing = &mut combined[position].1;
                existing.push_str(", ");
                append_header_value_string(existing, value);
            } else {
                positions.insert(name.clone(), combined.len());
                combined.push((name.clone(), header_value_string(value)));
            }
        }

        combined
    }

    /// Copies the raw fields into the HTTP transport representation.
    pub(super) fn to_header_map(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in &self.entries {
            headers.append(name.clone(), value.clone());
        }
        headers
    }
}
