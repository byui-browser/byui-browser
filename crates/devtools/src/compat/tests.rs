//! Audit tests against the real engine crates. JavaScript probes run in a
//! bare `js::Realm` (no Web APIs), which is enough to exercise detection and
//! the syntax probes; the shell's tests cover the full page realm.

use super::*;

/// Runs a probe page's inline scripts in one fresh realm.
struct BareRealm;

impl PageScriptEngine for BareRealm {
    fn evaluate_page(&self, html: &str) -> js::JsResult<js::Value> {
        let realm = js::Realm::new();
        let mut last = js::Value::Undefined;
        for source in html::parse_raw_html(html.to_owned()).script_sources() {
            last = realm.evaluate_script(&source)?;
        }
        Ok(last)
    }
}

fn audit(html: &str, stylesheets: &[&str], scripts: &[&str], error: Option<&str>) -> AuditReport {
    let stylesheets: Vec<String> = stylesheets.iter().map(|s| (*s).to_owned()).collect();
    let scripts: Vec<String> = scripts.iter().map(|s| (*s).to_owned()).collect();
    Auditor::new(BareRealm).audit(
        &PageSources {
            html,
            stylesheets: &stylesheets,
            scripts: &scripts,
        },
        error,
    )
}

fn has(report: &AuditReport, area: Area, feature: &str) -> bool {
    report
        .in_area(area)
        .any(|finding| finding.feature == feature)
}

#[test]
fn reports_unsupported_javascript_syntax_with_locations() {
    let script = "var total = 0;\nfor (var i = 0; i < 3; i++) { total += i; }\nvar kind = typeof total;\nvar add = (a) => a + 1;";
    let report = audit("<script>x</script>", &[], &[script], None);
    let for_loop = report
        .in_area(Area::JavaScriptSyntax)
        .find(|finding| finding.feature == "for (;;) loops")
        .expect("for loops are not implemented by the engine");
    assert_eq!(for_loop.location.as_deref(), Some("inline script #1:2"));
    assert_eq!(for_loop.evidence, Evidence::Probed);
    assert!(has(&report, Area::JavaScriptSyntax, "typeof operator"));
    // Supported syntax is checked but not reported.
    assert!(!has(
        &report,
        Area::JavaScriptSyntax,
        "Arrow functions (=>)"
    ));
    // The engine's own parser names the script it cannot run.
    assert!(has(
        &report,
        Area::ScriptExecution,
        "inline script #1 cannot be parsed"
    ));
}

#[test]
fn reports_missing_globals_but_not_the_pages_own_names() {
    let script = "function Map() {}\nvar seen = new Map();\nvar observer = new IntersectionObserver(cb);\nvar obj = { fetch: 1 };";
    let report = audit("<script>x</script>", &[], &[script], None);
    let missing = report
        .in_area(Area::JavaScriptApi)
        .find(|finding| finding.feature == "IntersectionObserver")
        .expect("a bare realm has no IntersectionObserver");
    assert_eq!(missing.location.as_deref(), Some("inline script #1:3"));
    assert!(
        !has(&report, Area::JavaScriptApi, "Map"),
        "page declares Map"
    );
    assert!(
        !has(&report, Area::JavaScriptApi, "fetch"),
        "object keys are not references"
    );
}

#[test]
fn reports_script_loading_problems_seen_in_the_page() {
    let html = "<script src=\"https://x.test/lib/app.js?v=2\"></script>\
                <script type=\"application/ld+json\">{\"a\": 1}</script>";
    let report = audit(
        html,
        &[],
        &["", "{\"a\": 1}"],
        Some("undefined variable: foo"),
    );
    assert!(has(&report, Area::ScriptExecution, "app.js failed to load"));
    assert!(has(
        &report,
        Area::ScriptExecution,
        "<script type=\"application/ld+json\">"
    ));
    assert!(has(
        &report,
        Area::ScriptExecution,
        "Page scripts stopped with an error"
    ));
}

#[test]
fn css_findings_come_from_render_probes() {
    let css = "@media (min-width: 600px) { .a { color: #ff0000; } }\n\
               .grid { display: grid; }\n\
               .box { width: 100px; margin: 10px; color: #00ff00; border-radius: 4px; }\n\
               a:hover { color: #0000ff; }\n\
               .sib + .sib { height: 10px; }";
    let report = audit("<div class=\"box\"></div>", &[css], &[], None);
    assert!(has(&report, Area::CssAtRule, "@media"));
    assert!(has(&report, Area::CssValue, "display: grid"));
    assert!(has(&report, Area::CssProperty, "border-radius"));
    assert!(has(&report, Area::CssSelector, ":hover"));
    let radius = report
        .in_area(Area::CssProperty)
        .find(|finding| finding.feature == "border-radius")
        .unwrap();
    assert_eq!(radius.location.as_deref(), Some("stylesheet #1:3"));
    // Implemented features are checked but not reported.
    for working in ["width", "margin", "color"] {
        assert!(!has(&report, Area::CssProperty, working), "{working} works");
    }
    assert!(!has(
        &report,
        Area::CssSelector,
        "Next-sibling combinator (+)"
    ));
    assert!(report.features_checked > report.findings.len());
}

#[test]
fn inline_styles_and_html_elements_are_audited() {
    let html = "<!DOCTYPE html><html><body>\n<p style=\"text-shadow: 1px 1px #000\">hi</p>\n<canvas></canvas><p>plain</p></body></html>";
    let report = audit(html, &[], &[], None);
    let shadow = report
        .in_area(Area::CssProperty)
        .find(|finding| finding.feature == "text-shadow")
        .expect("text-shadow has no effect");
    assert_eq!(
        shadow.location.as_deref(),
        Some("HTML line 2 (style attribute)")
    );
    let canvas = report
        .in_area(Area::Html)
        .find(|finding| finding.feature == "<canvas>")
        .expect("canvas is not rendered");
    assert_eq!(canvas.location.as_deref(), Some("HTML line 3"));
}

#[test]
fn unobservable_properties_are_listed_as_unverified() {
    let report = audit("<p></p>", &[".x { cursor: pointer; }"], &[], None);
    assert!(!has(&report, Area::CssProperty, "cursor"));
    assert!(
        report
            .unverified
            .iter()
            .any(|item| item.feature == "cursor")
    );
}

#[test]
fn repeated_audits_reuse_probe_results() {
    let auditor = Auditor::new(BareRealm);
    let sheets = [".a { border-radius: 2px; display: grid }".to_owned()];
    let scripts = ["var x = typeof 1;".to_owned()];
    let page = PageSources {
        html: "<script>x</script>",
        stylesheets: &sheets,
        scripts: &scripts,
    };
    let first = auditor.audit(&page, None);
    let started = std::time::Instant::now();
    let second = auditor.audit(&page, None);
    assert_eq!(first, second);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn findings_are_sorted_by_area_then_frequency() {
    let script = "var a = typeof 1; var b = typeof 2; var c = 1 ? 2 : 3;";
    let report = audit(
        "<script>x</script>",
        &[".a{border-radius:1px}"],
        &[script],
        None,
    );
    let areas: Vec<Area> = report.findings.iter().map(|finding| finding.area).collect();
    let mut sorted = areas.clone();
    sorted.sort();
    assert_eq!(areas, sorted);
    let syntax: Vec<_> = report.in_area(Area::JavaScriptSyntax).collect();
    assert!(
        syntax
            .windows(2)
            .all(|pair| pair[0].occurrences >= pair[1].occurrences)
    );
    assert_eq!(line_at("a\nb\nc", 2), 2);
}
