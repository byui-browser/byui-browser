//! Compatibility audit: lists the web platform features a page uses that
//! this engine does not implement yet.
//!
//! The audit has two halves:
//!
//! 1. **Detection** reads the page's HTML, stylesheets, and scripts and
//!    records which features they use (CSS properties, at-rules, selectors,
//!    JavaScript syntax, Web APIs, HTML elements) and where.
//! 2. **Probing** asks the engine whether each used feature works. CSS and
//!    HTML probes render small fixture documents through the real `css`,
//!    `layout`, and `paint` crates and compare display lists; JavaScript
//!    probes evaluate tiny scripts in a page realm supplied by the caller
//!    through [`PageScriptEngine`].
//!
//! Nothing is reported from a hard-coded "unsupported" list: when another
//! team ships a feature, its probe starts passing and it drops out of the
//! report. The few findings that cannot be probed are labelled
//! [`Evidence::Documented`] or [`Evidence::Observed`].
//!
//! Limitations: detection is lexical, not a full parse. JavaScript instance
//! members are matched by name ([`Evidence::NameMatch`]), CSS values are only
//! checked for the value features in this module's catalog, and dynamic
//! behavior (hover, animation over time, network-dependent resources) is not
//! exercised.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

mod markup;
mod render;
mod scan;
mod script;
mod style;

/// The loaded sources of one page, as the browser fetched them.
///
/// Mirrors `browser::Page` without depending on the `browser` crate (which
/// depends on this one).
#[derive(Debug, Clone, Copy)]
pub struct PageSources<'a> {
    /// The document's HTML source.
    pub html: &'a str,
    /// Stylesheet sources in document order: one per `<link rel=stylesheet>`
    /// that loaded and per `<style>` element.
    pub stylesheets: &'a [String],
    /// Script sources in document order, one per `<script>` element; a script
    /// whose `src` failed to load is an empty string.
    pub scripts: &'a [String],
}

/// Evaluates probe pages the way the browser evaluates real pages.
///
/// Implementations must install the same Web APIs a loaded page receives,
/// run each inline `<script>` of `html` in document order in one fresh
/// realm, and return the completion value of the last script. The realm must
/// be discarded afterwards: probes may define globals. Probe pages never
/// fetch subresources, and implementations should not perform network or
/// storage side effects that outlive the call.
pub trait PageScriptEngine {
    /// Runs the inline scripts of the probe page `html`.
    fn evaluate_page(&self, html: &str) -> js::JsResult<js::Value>;
}

/// Which part of the platform a [`Finding`] concerns. Variants are ordered
/// the way the report presents them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Area {
    /// Problems running the page's scripts at all: load failures, scripts
    /// the engine cannot parse, and runtime errors.
    ScriptExecution,
    /// JavaScript language syntax.
    JavaScriptSyntax,
    /// JavaScript built-ins and Web APIs.
    JavaScriptApi,
    /// HTML elements and attributes that need engine behavior.
    Html,
    /// Stylesheet content the CSS parser rejected.
    CssParsing,
    /// CSS properties with no observable effect.
    CssProperty,
    /// CSS value syntax such as `calc()`, units, and color functions.
    CssValue,
    /// CSS at-rules such as `@media`.
    CssAtRule,
    /// CSS selectors, pseudo-classes, and pseudo-elements.
    CssSelector,
}

impl Area {
    /// A short heading for this area, suitable for a report section.
    pub fn label(self) -> &'static str {
        match self {
            Self::ScriptExecution => "Script execution",
            Self::JavaScriptSyntax => "JavaScript syntax",
            Self::JavaScriptApi => "JavaScript & Web APIs",
            Self::Html => "HTML",
            Self::CssParsing => "CSS parsing",
            Self::CssProperty => "CSS properties",
            Self::CssValue => "CSS values",
            Self::CssAtRule => "CSS at-rules",
            Self::CssSelector => "CSS selectors",
        }
    }
}

/// How the audit concluded that a feature does not work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Evidence {
    /// A probe exercised the engine and the feature did not work.
    Probed,
    /// Seen directly in this page's load, such as a failed script fetch.
    Observed,
    /// The page uses a member with this name; which interface it belongs to
    /// was inferred from the name alone, so the page may be unaffected.
    NameMatch,
    /// Cannot be probed statically; the engine's own documentation states
    /// the limitation.
    Documented,
}

impl Evidence {
    /// A short tag describing this evidence for display.
    pub fn label(self) -> &'static str {
        match self {
            Self::Probed => "probed",
            Self::Observed => "observed",
            Self::NameMatch => "by name",
            Self::Documented => "documented",
        }
    }
}

/// One feature the page uses that does not work in this engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The platform area, used to group findings.
    pub area: Area,
    /// The feature, such as `display: grid` or `IntersectionObserver`.
    pub feature: String,
    /// What fails and, where useful, what the page sees instead.
    pub detail: String,
    /// How many times the page uses the feature (at least 1).
    pub occurrences: usize,
    /// The first place the page uses it, such as `app.js:14`.
    pub location: Option<String>,
    /// How the audit reached its conclusion.
    pub evidence: Evidence,
}

/// The result of auditing one page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditReport {
    /// Features that do not work, sorted by [`Area`] and then by how often
    /// the page uses them (most first).
    pub findings: Vec<Finding>,
    /// Number of distinct features the page uses that were checked,
    /// including the ones that work.
    pub features_checked: usize,
    /// Features the page uses that no static probe can check, such as
    /// `cursor` or `:checked`. They may or may not work.
    pub unverified: Vec<Unverified>,
}

/// A feature the page uses whose support the audit could not determine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unverified {
    /// The feature, such as `cursor` or `:checked`.
    pub feature: String,
    /// Why it could not be checked.
    pub reason: String,
    /// The first place the page uses it.
    pub location: Option<String>,
}

impl AuditReport {
    pub(crate) fn unverified_push(&mut self, feature: &str, location: Option<&str>, reason: &str) {
        self.unverified.push(Unverified {
            feature: feature.to_owned(),
            reason: reason.to_owned(),
            location: location.map(str::to_owned),
        });
    }

    /// Findings in `area`, in report order.
    pub fn in_area(&self, area: Area) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(move |finding| finding.area == area)
    }
}

/// Runs compatibility audits and remembers probe outcomes.
///
/// Probe results describe the engine, not the page, so one `Auditor` should
/// be kept for the browser session: later audits reuse earlier probes and run
/// much faster. Use a new `Auditor` if the engine itself changes.
pub struct Auditor<E> {
    engine: E,
    cache: Mutex<HashMap<String, Verdict>>,
}

/// The cached outcome of one probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Works,
    Broken,
    /// The probe's own reference did not render, so it proves nothing.
    Inconclusive,
}

impl From<bool> for Verdict {
    fn from(works: bool) -> Self {
        if works { Self::Works } else { Self::Broken }
    }
}

impl<E: PageScriptEngine> Auditor<E> {
    /// Creates an auditor that probes JavaScript through `engine`.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Audits `page` and lists the features it uses that do not work.
    ///
    /// `script_error` is the error that stopped the page's own script run, if
    /// any, as the browser observed it. This may take a moment on a cold
    /// cache (each probe boots a realm or renders a fixture); call it off the
    /// UI thread.
    pub fn audit(&self, page: &PageSources<'_>, script_error: Option<&str>) -> AuditReport {
        let started = std::time::Instant::now();
        let map = markup::PageMap::new(page);
        let mut report = AuditReport::default();
        script::audit(self, page, &map, script_error, &mut report);
        markup::audit(self, &map, &mut report);
        style::audit(self, page, &map, &mut report);
        report.findings.sort_by(|a, b| {
            a.area
                .cmp(&b.area)
                .then(b.occurrences.cmp(&a.occurrences))
                .then_with(|| a.feature.cmp(&b.feature))
        });
        log::debug!(
            "compatibility audit: {} findings from {} features in {:?}",
            report.findings.len(),
            report.features_checked,
            started.elapsed()
        );
        report
    }

    /// Returns the cached verdict for `key`, computing it with `probe` once.
    ///
    /// The lock is not held while probing, so parallel callers may race to
    /// compute the same key; both reach the same answer.
    fn verdict(&self, key: &str, probe: impl FnOnce() -> Verdict) -> Verdict {
        if let Some(&known) = self.cache.lock().expect("probe cache lock").get(key) {
            return known;
        }
        let outcome = probe();
        self.cache
            .lock()
            .expect("probe cache lock")
            .insert(key.to_owned(), outcome);
        outcome
    }

    /// The cached verdict for `key`, if it has been probed.
    fn known(&self, key: &str) -> Option<Verdict> {
        self.cache
            .lock()
            .expect("probe cache lock")
            .get(key)
            .copied()
    }

    /// Stores a verdict computed elsewhere (for example on worker threads).
    fn remember(&self, key: String, verdict: Verdict) {
        self.cache
            .lock()
            .expect("probe cache lock")
            .insert(key, verdict);
    }

    /// Like [`Self::verdict`] for probes that are always conclusive.
    fn cached(&self, key: &str, probe: impl FnOnce() -> bool) -> bool {
        self.verdict(key, || probe().into()) == Verdict::Works
    }
}

/// Tallies how often each feature is used and where it first appears.
#[derive(Debug, Default)]
pub(crate) struct Usage {
    entries: BTreeMap<String, Use>,
}

#[derive(Debug, Default)]
pub(crate) struct Use {
    pub count: usize,
    pub location: Option<String>,
    /// Distinct example values, for features that have them (CSS).
    pub values: Vec<String>,
}

impl Usage {
    /// Records one use of `feature` at `location`.
    pub fn add(&mut self, feature: &str, location: impl FnOnce() -> String) -> &mut Use {
        let entry = self.entries.entry(feature.to_owned()).or_default();
        entry.count += 1;
        if entry.location.is_none() {
            entry.location = Some(location());
        }
        entry
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Use)> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

impl Use {
    /// Remembers `value` as an example, keeping at most `limit` distinct ones.
    pub fn remember(&mut self, value: &str, limit: usize) {
        if self.values.len() < limit && !self.values.iter().any(|known| known == value) {
            self.values.push(value.to_owned());
        }
    }
}

impl Finding {
    pub(crate) fn new(
        area: Area,
        feature: impl Into<String>,
        detail: impl Into<String>,
        evidence: Evidence,
    ) -> Self {
        Self {
            area,
            feature: feature.into(),
            detail: detail.into(),
            occurrences: 1,
            location: None,
            evidence,
        }
    }

    pub(crate) fn at(mut self, usage: &Use) -> Self {
        self.occurrences = usage.count.max(1);
        self.location = usage.location.clone();
        self
    }
}

/// The 1-based line containing byte `offset` of `text`.
pub(crate) fn line_at(text: &str, offset: usize) -> usize {
    let end = offset.min(text.len());
    let end = (0..=end)
        .rev()
        .find(|&index| text.is_char_boundary(index))
        .unwrap_or(0);
    text[..end].bytes().filter(|&byte| byte == b'\n').count() + 1
}

#[cfg(test)]
mod tests;
