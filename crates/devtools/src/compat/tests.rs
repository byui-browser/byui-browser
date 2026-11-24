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
    let script = "var total = 0;\nfor (var i = 0; i < 3; i++) { total += i; }\nvar kind = /x/.test('x');\nvar add = (a) => a + 1;";
    let report = audit("<script>x</script>", &[], &[script], None);
    let for_loop = report
        .in_area(Area::JavaScriptSyntax)
        .find(|finding| finding.feature == "for (;;) loops")
        .expect("for loops are not implemented by the engine");
    assert_eq!(for_loop.location.as_deref(), Some("inline script #1:2"));
    assert_eq!(for_loop.evidence, Evidence::Probed);
    assert!(!has(
        &report,
        Area::JavaScriptSyntax,
        "Regular expression literals"
    ));
    // Supported syntax is checked but not reported.
    assert!(!has(
        &report,
        Area::JavaScriptSyntax,
        "Arrow functions (=>)"
    ));
    // Regex literals are accepted by the parser; the remaining for-loop gap
    // is reported by the syntax probe above.
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
               .box { width: 100px; margin: 10px; color: #00ff00; text-shadow: 1px 1px #000; border-radius: 4px; box-shadow: 2px 2px #000; }\n\
               a:hover { color: #0000ff; }\n\
               .sib + .sib { height: 10px; }";
    let report = audit("<div class=\"box\"></div>", &[css], &[], None);
    assert!(!has(&report, Area::CssAtRule, "@media"));
    assert!(has(&report, Area::CssValue, "display: grid"));
    assert!(has(&report, Area::CssProperty, "text-shadow"));
    assert!(has(&report, Area::CssSelector, ":hover"));
    let shadow = report
        .in_area(Area::CssProperty)
        .find(|finding| finding.feature == "text-shadow")
        .unwrap();
    assert_eq!(shadow.location.as_deref(), Some("stylesheet #1:3"));
    // Implemented features are checked but not reported.
    for working in ["width", "margin", "color", "border-radius", "box-shadow"] {
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
        Some("HTML markup (style attribute)")
    );
    let canvas = report
        .in_area(Area::Html)
        .find(|finding| finding.feature == "<canvas>")
        .expect("canvas is not rendered");
    assert_eq!(canvas.location.as_deref(), Some("HTML markup"));
}

#[test]
fn a_drop_down_select_showing_its_selected_option_is_supported() {
    let html = "<!DOCTYPE html><html><body>\
                <select><option>One</option><option selected>Two</option></select></body></html>";
    let report = audit(html, &[], &[], None);
    assert!(!has(&report, Area::Html, "<select>"));
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
    let sheets = [".a { text-shadow: 1px 1px #000; display: grid }".to_owned()];
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
        &[".a{text-shadow:1px 1px #000}"],
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

#[test]
fn parse_failures_carry_a_preview_of_the_failing_line() {
    let script = "var a = 1;\nvar b = 2;\nvar c = ;\nvar d = 4;";
    let error = js::parse(script).expect_err("`var c = ;` is not valid");
    let run_error = format!("{error}");
    let report = audit("<script>x</script>", &[], &[script], Some(&run_error));
    let failure = report
        .findings
        .iter()
        .find(|finding| finding.feature == "inline script #1 cannot be parsed")
        .expect("the script cannot be parsed");
    let preview = failure.preview.as_ref().expect("parse failures show code");
    assert_eq!(preview.source, "inline script #1");
    assert_eq!(preview.line, 3);
    assert_eq!(preview.column, error.column);
    assert_eq!(preview.first_line, 1);
    assert_eq!(preview.lines.len(), 4, "the whole short script fits");
    assert_eq!(preview.focus(), "var c = ;");
    // The observed run error points at the same spot, so it shares the preview.
    let stopped = report
        .findings
        .iter()
        .find(|finding| finding.feature == "Page scripts stopped with an error")
        .unwrap();
    assert_eq!(stopped.preview.as_ref(), Some(preview));
}

#[test]
fn located_findings_get_previews_and_unlocated_ones_do_not() {
    let script = "var total = 0;\nfor (var i = 0; i < 3; i++) { total += i; }";
    let css = ".a { color: red; }\n.box { text-shadow: 1px 1px #000; }";
    let report = audit("<div class=\"box\"></div>", &[css], &[script], None);
    let for_loop = report
        .findings
        .iter()
        .find(|finding| finding.feature == "for (;;) loops")
        .unwrap();
    let preview = for_loop.preview.as_ref().expect("script line is known");
    assert_eq!(preview.line, 2);
    assert_eq!(preview.column, None);
    assert!(preview.focus().starts_with("for (var i"));
    let shadow = report
        .findings
        .iter()
        .find(|finding| finding.feature == "text-shadow")
        .unwrap();
    assert_eq!(
        shadow.preview.as_ref().map(SourcePreview::focus),
        Some(".box { text-shadow: 1px 1px #000; }")
    );
}

#[test]
fn data_block_scripts_preview_their_first_lines() {
    let report = audit(
        "<script type=\"application/ld+json\">{\"a\": 1}</script>",
        &[],
        &["{\"a\": 1}"],
        None,
    );
    let block = report
        .findings
        .iter()
        .find(|finding| finding.feature == "<script type=\"application/ld+json\">")
        .unwrap();
    assert_eq!(
        block.preview.as_ref().map(SourcePreview::focus),
        Some("{\"a\": 1}")
    );
}

#[test]
fn previews_window_long_minified_lines_around_the_column() {
    let line = format!("{}BAD{}", "x".repeat(10_000), "y".repeat(10_000));
    let preview = SourcePreview::new("min.js", &line, 1, Some(10_001)).unwrap();
    assert_eq!(preview.first_line, 1);
    assert_eq!(preview.lines.len(), 1);
    assert!(preview.lines[0].chars().count() <= SourcePreview::MAX_COLUMNS);
    let at = preview.column.unwrap() - preview.first_column;
    assert!(preview.lines[0][at..].starts_with("BAD"));
    assert!(SourcePreview::new("min.js", &line, 2, None).is_none());
    // Tabs are shown as single spaces so columns stay accurate.
    let tabbed = SourcePreview::new("t.js", "\tvar a;", 1, Some(2)).unwrap();
    assert_eq!(tabbed.focus(), " var a;");
}

#[test]
fn supported_css_properties_are_not_reported_as_missing() {
    let report = audit(
        "<div>example</div>",
        &[
            "div { animation: grow 1s linear; min-height:40px; min-width:100px; text-overflow:ellipsis; visibility:hidden } @keyframes grow { from { height:10px } to { height:40px } }",
        ],
        &[],
        None,
    );
    for property in [
        "animation",
        "min-height",
        "min-width",
        "text-overflow",
        "visibility",
    ] {
        assert!(
            !has(&report, Area::CssProperty, property),
            "{property}: {:?}",
            report.findings
        );
    }
    assert!(has(
        &report,
        Area::CssValue,
        "text-overflow: ellipsis limitations"
    ));
    assert!(has(
        &report,
        Area::CssValue,
        "Animation timeline limitations"
    ));
}

#[test]
fn partial_css_support_reports_the_actual_value_gaps_with_locations() {
    let report = audit(
        "",
        &[
            ".a { visibility:collapse; text-overflow:'..'; min-width:min-content; overflow:auto }
.b { text-overflow:clip ellipsis; min-height:max-content }",
        ],
        &[],
        None,
    );
    for feature in [
        "visibility: collapse",
        "text-overflow strings / two-value syntax",
        "Intrinsic minimum sizes",
        "Overflow scrolling limitations",
    ] {
        let finding = report
            .in_area(Area::CssValue)
            .find(|f| f.feature == feature)
            .unwrap_or_else(|| panic!("missing {feature}"));
        assert_eq!(finding.evidence, Evidence::Documented);
        assert!(finding.location.is_some());
    }
    let intrinsic = report
        .in_area(Area::CssValue)
        .find(|f| f.feature == "Intrinsic minimum sizes")
        .unwrap();
    assert_eq!(intrinsic.occurrences, 2);
}

#[test]
fn ordinary_css_values_do_not_trigger_partial_support_warnings() {
    let report = audit(
        "",
        &[
            ".a { visibility:visible; min-width:20px; min-height:30%; text-overflow:clip; animation:none; overflow:hidden }",
        ],
        &[],
        None,
    );
    for feature in [
        "visibility: collapse",
        "text-overflow strings / two-value syntax",
        "Intrinsic minimum sizes",
        "text-overflow: ellipsis limitations",
        "Animation timeline limitations",
        "Overflow scrolling limitations",
    ] {
        assert!(!has(&report, Area::CssValue, feature), "{feature}");
    }
}

#[test]
fn new_css_layout_and_paint_properties_are_not_flagged_as_missing() {
    let report = audit(
        "",
        &[
            ".a {word-break:break-all;padding-inline-end:20px;outline-offset:5px;max-block-size:30px;text-rendering:optimizeSpeed}",
        ],
        &[],
        None,
    );
    for property in [
        "word-break",
        "padding-inline-end",
        "outline-offset",
        "max-block-size",
        "text-rendering",
    ] {
        assert!(
            !has(&report, Area::CssProperty, property),
            "{property}: {:?}",
            report.findings
        );
    }
}

#[test]
fn visual_paint_properties_are_not_flagged_as_missing() {
    let report = audit(
        "",
        &[
            ".a {background:red;opacity:0.5;border-color:red;font:bold 12px serif;overflow:hidden;z-index:2;\
             -webkit-box-shadow:2px 2px 3px red;vertical-align:super;font-family:monospace;\
             font-weight:bold;-ms-transform:rotate(45deg);-webkit-transform:rotate(45deg);\
             font-style:italic;transform:rotate(45deg)}",
        ],
        &[],
        None,
    );
    for property in [
        "background",
        "opacity",
        "border-color",
        "-webkit-box-shadow",
        "vertical-align",
        "font-family",
        "font",
        "overflow",
        "z-index",
        "font-weight",
        "-ms-transform",
        "-webkit-transform",
        "font-style",
        "transform",
    ] {
        assert!(
            !has(&report, Area::CssProperty, property),
            "{property}: {:?}",
            report.findings
        );
    }
}

#[test]
fn interaction_and_text_quality_limitations_are_reported_precisely() {
    let report = audit(
        "",
        &[
            ".a {cursor:url(cursor.png), pointer;user-select:contain;text-rendering:optimizeLegibility;word-break:auto-phrase}",
        ],
        &[],
        None,
    );
    for feature in [
        "Image cursors",
        "user-select: contain",
        "Text rendering quality limitations",
        "Advanced word breaking",
    ] {
        assert!(has(&report, Area::CssValue, feature), "{feature}");
    }
}

#[test]
fn logical_sizing_and_clear_properties_are_not_flagged_as_missing() {
    let report = audit(
        "",
        &[
            ".a {margin-block:4px;inline-size:40px;border-left-color:red;max-width:30px;block-size:20px;padding-inline:3px;clear:both;min-inline-size:10px;padding-block:2px}",
        ],
        &[],
        None,
    );
    for property in [
        "margin-block",
        "inline-size",
        "border-left-color",
        "max-width",
        "block-size",
        "padding-inline",
        "clear",
        "min-inline-size",
        "padding-block",
    ] {
        assert!(
            !has(&report, Area::CssProperty, property),
            "{property}: {:?}",
            report.findings
        );
    }
}
