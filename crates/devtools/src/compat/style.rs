//! CSS audit: parse errors, properties, value syntax, at-rules, and
//! selectors, each probed by rendering the shared fixture.

use std::sync::atomic::{AtomicUsize, Ordering};

use css::Declaration;

use super::markup::PageMap;
use super::render::{Rendered, render_fixture};
use super::{
    Area, AuditReport, Auditor, Evidence, Finding, PageScriptEngine, PageSources, Usage, line_at,
};

/// Base stylesheets that put the target `#t` in different formatting
/// contexts, so properties that only matter in one (like `gap` or
/// `flex-grow`) still get a chance to show an effect.
const CONTEXTS: &[&str] = &[
    "",
    "#t{display:flex}",
    "#p{display:flex}",
    "#t{display:grid}",
    "#p{display:grid}",
    "#t{position:relative}",
    "#t{width:60px;height:30px;padding:7px;overflow:hidden;white-space:nowrap}",
    "#t{display:list-item}",
];

/// Values tried in the plain context when the page's own values show no
/// effect. Covers the common value types: lengths, colors, keywords, and
/// numbers.
const CANDIDATES: &[&str] = &[
    "none",
    "0",
    "10px",
    "33px",
    "50%",
    "3em",
    "auto",
    "#ff0000",
    "transparent",
    "block",
    "inline-block",
    "flex",
    "grid",
    "hidden",
    "absolute",
    "relative",
    "fixed",
    "bold",
    "italic",
    "center",
    "right",
    "end",
    "space-between",
    "column",
    "row-reverse",
    "wrap",
    "nowrap",
    "pre",
    "uppercase",
    "underline",
    "2",
    "0.5",
    "4",
    "-1",
    "1fr 1fr",
    "40px 40px",
    "4px solid #ff0000",
    "border-box",
    "left",
    "both",
    "rotate(45deg)",
    "scale(2)",
    "2px 2px 4px #ff0000",
    "serif",
    "monospace",
    "ellipsis",
    "break-all",
    "vertical-rl",
    "rtl",
    "stretch",
    "baseline",
    "small-caps",
    "dotted",
    "square",
    "lower-roman",
    "inside",
    "cover",
    "no-repeat",
    "scroll",
    "100px",
    "max-content",
    "clip",
    "contain",
    "sideways",
];

/// A shorter list tried in every other context.
const CONTEXT_CANDIDATES: &[&str] = &[
    "10px",
    "50%",
    "auto",
    "center",
    "2",
    "none",
    "column",
    "wrap",
    "1fr 1fr",
    "space-between",
    "end",
    "stretch",
    "-1",
    "#ff0000",
];

/// Properties whose effect is interactive, temporal, or print-only, so a
/// static render cannot show whether they work.
const UNOBSERVABLE: &[&str] = &[
    "cursor",
    "pointer-events",
    "user-select",
    "-webkit-user-select",
    "-moz-user-select",
    "-ms-user-select",
    "touch-action",
    "will-change",
    "scroll-behavior",
    "caret-color",
    "accent-color",
    "resize",
    "transition",
    "transition-property",
    "transition-duration",
    "transition-timing-function",
    "transition-delay",
    "transition-behavior",
    "-webkit-transition",
    "-webkit-tap-highlight-color",
    "-webkit-font-smoothing",
    "-moz-osx-font-smoothing",
    "text-rendering",
    "image-rendering",
    "speak",
    "print-color-adjust",
    "-webkit-print-color-adjust",
    "color-scheme",
    "forced-color-adjust",
    "page-break-before",
    "page-break-after",
    "page-break-inside",
    "break-before",
    "break-after",
    "break-inside",
    "orphans",
    "widows",
    "scroll-snap-type",
    "scroll-snap-align",
    "scroll-snap-stop",
    "scroll-margin",
    "scroll-margin-top",
    "scroll-padding",
    "scroll-padding-top",
    "overscroll-behavior",
    "overscroll-behavior-x",
    "overscroll-behavior-y",
    "scrollbar-width",
    "scrollbar-color",
    "scrollbar-gutter",
    "content-visibility",
    "contain-intrinsic-size",
    "backface-visibility",
    "-webkit-backface-visibility",
    "animation-play-state",
    "font-display",
    "unicode-range",
    "src",
    "size",
    "interpolate-size",
    "view-transition-name",
    "-webkit-text-size-adjust",
    "text-size-adjust",
    "-ms-text-size-adjust",
    "zoom",
];

use super::Verdict as Outcome;

/// Whether `test` renders exactly like `reference`, which itself must
/// differ from the plain fixture.
fn same(test: &str, reference: &str) -> Outcome {
    let (Some(base), Some(reference)) = (render_fixture(""), render_fixture(reference)) else {
        return Outcome::Inconclusive;
    };
    if reference.list == base.list {
        return Outcome::Inconclusive;
    }
    render_fixture(test)
        .is_some_and(|test| test.list == reference.list)
        .into()
}

/// Whether `css` renders differently from `reference` (default: the plain
/// fixture).
fn differs(css: &str, reference: &str) -> Outcome {
    match (render_fixture(css), render_fixture(reference)) {
        (Some(test), Some(reference)) => (test.list != reference.list).into(),
        _ => Outcome::Broken,
    }
}

/// Whether `css` renders and passes `check`.
fn fixture(css: &str, check: impl FnOnce(&Rendered) -> bool) -> Outcome {
    render_fixture(css).is_some_and(|page| check(&page)).into()
}

/// Whether `value` paints `#t`'s background within a few units of `rgba`.
fn paints_color(value: &str, rgba: [u8; 4]) -> Outcome {
    fixture(&format!("#t{{background-color:{value}}}"), |page| {
        page.fill_color("t").is_some_and(|color| {
            [color.r, color.g, color.b, color.a]
                .iter()
                .zip(rgba)
                .all(|(&got, want)| got.abs_diff(want) <= 6)
        })
    })
}

/// Whether `selector` matches the fixture element `#target`.
fn selects(selector: &str, target: &str) -> Outcome {
    fixture(&format!("{selector}{{background-color:#fa0000}}"), |page| {
        page.fill_color(target)
            .is_some_and(|color| (color.r, color.g, color.b) == (0xfa, 0, 0))
    })
}

fn same_row(page: &Rendered, a: &str, b: &str) -> bool {
    matches!((page.rect(a), page.rect(b)), (Some(a), Some(b)) if (a.y - b.y).abs() < 0.5 && b.x > a.x)
}

fn base_rect(id: &str) -> Option<layout::Rect> {
    render_fixture("").and_then(|page| page.rect(id))
}

/// A CSS feature found by looking at declarations.
struct ValueFeature {
    name: &'static str,
    /// Whether a declaration (lowercased property, lowercased value) uses it.
    detect: fn(&str, &str) -> bool,
    check: fn() -> Outcome,
}

/// Whether `value` calls the CSS function `name(`.
fn calls(value: &str, name: &str) -> bool {
    value.match_indices(name).any(|(index, _)| {
        value[index + name.len()..].starts_with('(')
            && !value[..index]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// Whether `value` has a number followed by `unit`.
fn has_unit(value: &str, unit: &str) -> bool {
    value.match_indices(unit).any(|(index, _)| {
        value[..index]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_digit() || c == '.')
            && !value[index + unit.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

fn has_hex(value: &str, lengths: &[usize]) -> bool {
    value.match_indices('#').any(|(index, _)| {
        let digits = value[index + 1..]
            .chars()
            .take_while(char::is_ascii_hexdigit)
            .count();
        lengths.contains(&digits)
            && !value[index + 1 + digits..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

fn is_display(property: &str, value: &str, keywords: &[&str]) -> bool {
    property == "display"
        && value
            .split_whitespace()
            .any(|word| keywords.contains(&word))
}

const VALUE_FEATURES: &[ValueFeature] = &[
    ValueFeature {
        name: "calc()",
        detect: |_, v| calls(v, "calc"),
        check: || same("#t{width:calc(20px + 30px)}", "#t{width:50px}"),
    },
    ValueFeature {
        name: "min() / max()",
        detect: |_, v| calls(v, "min") || calls(v, "max"),
        check: || same("#t{width:min(50px, 80px)}", "#t{width:50px}"),
    },
    ValueFeature {
        name: "clamp()",
        detect: |_, v| calls(v, "clamp"),
        check: || same("#t{width:clamp(10px, 50px, 90px)}", "#t{width:50px}"),
    },
    ValueFeature {
        name: "Custom properties (var())",
        detect: |_, v| calls(v, "var"),
        check: || same("#t{--probe:50px;width:var(--probe)}", "#t{width:50px}"),
    },
    ValueFeature {
        name: "rem units",
        detect: |_, v| has_unit(v, "rem"),
        check: || same("#t{width:2rem}", "#t{width:32px}"),
    },
    ValueFeature {
        name: "em units",
        detect: |_, v| has_unit(v, "em"),
        check: || {
            same(
                "#t{font-size:20px;width:2em}",
                "#t{font-size:20px;width:40px}",
            )
        },
    },
    ValueFeature {
        name: "Viewport units (vw, vh, vmin, vmax)",
        detect: |_, v| ["vw", "vh", "vmin", "vmax"].iter().any(|u| has_unit(v, u)),
        check: || same("#t{width:10vw;height:10vh}", "#t{width:80px;height:60px}"),
    },
    ValueFeature {
        name: "Dynamic viewport units (dvh, svh, lvh, …)",
        detect: |_, v| {
            ["dvh", "svh", "lvh", "dvw", "svw", "lvw"]
                .iter()
                .any(|u| has_unit(v, u))
        },
        check: || same("#t{height:10dvh}", "#t{height:60px}"),
    },
    ValueFeature {
        name: "Absolute units (pt, pc, in, cm, mm)",
        detect: |_, v| {
            ["pt", "pc", "in", "cm", "mm"]
                .iter()
                .any(|u| has_unit(v, u))
        },
        check: || same("#t{width:72pt;height:1in}", "#t{width:96px;height:96px}"),
    },
    ValueFeature {
        name: "ch units",
        detect: |_, v| has_unit(v, "ch"),
        check: || same("#t{width:2ch}", "#t{width:16px}"),
    },
    ValueFeature {
        name: "Short hex colors (#rgb)",
        detect: |_, v| has_hex(v, &[3]),
        check: || paints_color("#f00", [255, 0, 0, 255]),
    },
    ValueFeature {
        name: "Hex colors with alpha (#rrggbbaa)",
        detect: |_, v| has_hex(v, &[4, 8]),
        check: || paints_color("#ff000080", [255, 0, 0, 128]),
    },
    ValueFeature {
        name: "rgb()",
        detect: |_, v| calls(v, "rgb"),
        check: || paints_color("rgb(255, 0, 0)", [255, 0, 0, 255]),
    },
    ValueFeature {
        name: "Translucent colors (rgba(), alpha < 1)",
        detect: |_, v| {
            let slash_alpha = ["rgb", "hsl", "hwb", "lab", "lch", "oklab", "oklch", "color"]
                .iter()
                .any(|f| calls(v, f))
                && v.contains('/');
            calls(v, "rgba") || calls(v, "hsla") || slash_alpha || has_hex(v, &[4, 8])
        },
        check: || paints_color("rgba(255, 0, 0, 0.5)", [255, 0, 0, 128]),
    },
    ValueFeature {
        name: "Space-separated rgb() (rgb(255 0 0))",
        detect: |_, v| {
            (calls(v, "rgb") || calls(v, "rgba"))
                && v.split("rgb")
                    .skip(1)
                    .any(|call| !call.split(')').next().unwrap_or_default().contains(','))
        },
        check: || paints_color("rgb(255 0 0)", [255, 0, 0, 255]),
    },
    ValueFeature {
        name: "hsl() / hsla()",
        detect: |_, v| calls(v, "hsl") || calls(v, "hsla"),
        check: || paints_color("hsl(0, 100%, 50%)", [255, 0, 0, 255]),
    },
    ValueFeature {
        name: "hwb()",
        detect: |_, v| calls(v, "hwb"),
        check: || paints_color("hwb(0 0% 0%)", [255, 0, 0, 255]),
    },
    ValueFeature {
        name: "lab() / lch() / oklab() / oklch()",
        detect: |_, v| ["lab", "lch", "oklab", "oklch"].iter().any(|f| calls(v, f)),
        check: || paints_color("oklch(62.8% 0.2577 29.23)", [255, 0, 0, 255]),
    },
    ValueFeature {
        name: "color()",
        detect: |_, v| calls(v, "color"),
        check: || paints_color("color(srgb 1 0 0)", [255, 0, 0, 255]),
    },
    ValueFeature {
        name: "color-mix()",
        detect: |_, v| calls(v, "color-mix"),
        check: || {
            paints_color(
                "color-mix(in srgb, #ff0000 50%, #0000ff)",
                [128, 0, 128, 255],
            )
        },
    },
    ValueFeature {
        name: "currentcolor",
        detect: |_, v| v.contains("currentcolor"),
        check: || {
            fixture("#t{color:#ff0000;background-color:currentcolor}", |page| {
                page.fill_color("t")
                    .is_some_and(|c| (c.r, c.g, c.b) == (255, 0, 0))
            })
        },
    },
    ValueFeature {
        name: "Gradients (linear-gradient(), …)",
        detect: |_, v| v.contains("gradient("),
        check: || differs("#t{background-image:linear-gradient(#ff0000, #0000ff)}", ""),
    },
    ValueFeature {
        name: "!important",
        detect: |_, v| v.ends_with("!important"),
        check: || {
            same(
                "#t{height:40px !important}#t{height:20px}",
                "#t{height:40px}",
            )
        },
    },
    ValueFeature {
        name: "inherit",
        detect: |_, v| v == "inherit",
        check: || {
            same(
                "#p{background-color:#ff0000}#t{background-color:inherit}",
                "#p{background-color:#ff0000}#t{background-color:#ff0000}",
            )
        },
    },
    ValueFeature {
        name: "initial",
        detect: |_, v| v == "initial",
        check: || {
            same(
                "#p{font-size:30px}#t{font-size:initial}",
                "#p{font-size:30px}#t{font-size:16px}",
            )
        },
    },
    ValueFeature {
        name: "unset",
        detect: |_, v| v == "unset",
        check: || {
            same(
                "#p{color:#ff0000}#t{color:#0000ff}#t{color:unset}",
                "#p{color:#ff0000}#t{color:#ff0000}",
            )
        },
    },
    ValueFeature {
        name: "display: flex",
        detect: |p, v| is_display(p, v, &["flex"]),
        check: || fixture("#t{display:flex}", |page| same_row(page, "c", "d")),
    },
    ValueFeature {
        name: "display: inline-flex",
        detect: |p, v| is_display(p, v, &["inline-flex"]),
        check: || {
            fixture("#t{display:inline-flex}", |page| {
                same_row(page, "c", "d") && page.rect("t").is_some_and(|t| t.width < 700.0)
            })
        },
    },
    ValueFeature {
        name: "display: grid",
        detect: |p, v| is_display(p, v, &["grid", "inline-grid"]),
        check: || {
            fixture(
                "#t{display:grid;grid-template-columns:100px 100px}",
                |page| {
                    same_row(page, "c", "d")
                        && matches!((page.rect("c"), page.rect("d")), (Some(c), Some(d)) if (d.x - c.x - 100.0).abs() < 0.5)
                },
            )
        },
    },
    ValueFeature {
        name: "display: table / table-cell",
        detect: |p, v| is_display(p, v, &["table", "inline-table", "table-cell", "table-row"]),
        check: || {
            fixture("#t{display:table}#c,#d{display:table-cell}", |page| {
                same_row(page, "c", "d")
            })
        },
    },
    ValueFeature {
        name: "display: list-item",
        detect: |p, v| is_display(p, v, &["list-item"]),
        check: || differs("#t{display:list-item}", "#t{display:block}"),
    },
    ValueFeature {
        name: "display: contents",
        detect: |p, v| is_display(p, v, &["contents"]),
        check: || {
            fixture("#t{display:contents}", |page| {
                page.rect("t").is_none() && page.rect("c").is_some()
            })
        },
    },
    ValueFeature {
        name: "display: inline-block",
        detect: |p, v| is_display(p, v, &["inline-block"]),
        check: || {
            fixture("#t{display:inline-block}", |page| {
                page.rect("t").is_some_and(|t| t.width < 700.0)
            })
        },
    },
    ValueFeature {
        name: "display: none",
        detect: |p, v| is_display(p, v, &["none"]),
        check: || fixture("#t{display:none}", |page| page.rect("t").is_none()),
    },
    ValueFeature {
        name: "position: relative offsets",
        detect: |p, v| p == "position" && v == "relative",
        check: || {
            let base = base_rect("t");
            fixture(
                "#t{position:relative;top:10px}",
                |page| matches!((base, page.rect("t")), (Some(base), Some(t)) if (t.y - base.y - 10.0).abs() < 0.5),
            )
        },
    },
    ValueFeature {
        name: "position: absolute",
        detect: |p, v| p == "position" && v == "absolute",
        check: || {
            let base = base_rect("t");
            fixture(
                "#t{position:absolute;top:0;left:0;width:100px}",
                |page| matches!((base, page.rect("n")), (Some(base), Some(n)) if (n.y - base.y).abs() < 0.5),
            )
        },
    },
    ValueFeature {
        name: "position: fixed",
        detect: |p, v| p == "position" && v == "fixed",
        check: || {
            fixture("#t{position:fixed;top:5px;left:5px}", |page| {
                page.rect("t")
                    .is_some_and(|t| (t.x - 5.0).abs() < 0.5 && (t.y - 5.0).abs() < 0.5)
            })
        },
    },
    ValueFeature {
        name: "Floats",
        detect: |p, v| p == "float" && (v == "left" || v == "right" || v == "inline-start"),
        check: || {
            fixture("#c{float:left;width:50px}", |page| {
                match (page.rect("c"), page.text_position("child two")) {
                    (Some(c), Some((x, _))) => x >= c.x + 50.0,
                    _ => false,
                }
            })
        },
    },
    ValueFeature {
        name: "Intrinsic sizes (min-content, max-content, fit-content)",
        detect: |_, v| {
            v.contains("min-content") || v.contains("max-content") || v.contains("fit-content")
        },
        check: || {
            fixture("#t{width:max-content}", |page| {
                page.rect("t").is_some_and(|t| t.width < 700.0)
            })
        },
    },
];

struct AtRuleProbe {
    names: &'static [&'static str],
    feature: &'static str,
    check: fn() -> Outcome,
}

const AT_RULES: &[AtRuleProbe] = &[
    AtRuleProbe {
        names: &["media"],
        feature: "@media",
        check: || match same(
            "@media (min-width: 1px){#t{height:40px}}",
            "#t{height:40px}",
        ) {
            Outcome::Works => same(
                "@media (max-width: 1px){#t{height:40px}}#t{height:20px}",
                "#t{height:20px}",
            ),
            other => other,
        },
    },
    AtRuleProbe {
        names: &["supports"],
        feature: "@supports",
        check: || {
            same(
                "@supports (display: block){#t{height:40px}}",
                "#t{height:40px}",
            )
        },
    },
    AtRuleProbe {
        names: &["layer"],
        feature: "@layer",
        check: || same("@layer probe{#t{height:40px}}", "#t{height:40px}"),
    },
    AtRuleProbe {
        names: &["container"],
        feature: "@container",
        check: || {
            same(
                "#p{container-type:inline-size}@container (min-width: 1px){#t{height:40px}}",
                "#p{container-type:inline-size}#t{height:40px}",
            )
        },
    },
    AtRuleProbe {
        names: &["scope"],
        feature: "@scope",
        check: || same("@scope (#p){#t{height:40px}}", "#t{height:40px}"),
    },
    AtRuleProbe {
        names: &["keyframes", "-webkit-keyframes", "-moz-keyframes"],
        feature: "@keyframes animations",
        check: || {
            same(
                "@keyframes probe{from{height:40px}to{height:40px}}#t{animation:probe 10s infinite}",
                "#t{height:40px}",
            )
        },
    },
    AtRuleProbe {
        names: &["property"],
        feature: "@property",
        check: || {
            same(
                "@property --probe{syntax:'<length>';inherits:false;initial-value:50px}#t{width:var(--probe)}",
                "#t{width:50px}",
            )
        },
    },
];

/// At-rules with nothing to observe in a static render.
const IGNORED_AT_RULES: &[&str] = &["charset"];

struct SelectorProbe {
    /// Pseudo-class or pseudo-element names (without colons), or a
    /// combinator/attribute-operator token, that use the feature.
    uses: &'static [&'static str],
    feature: &'static str,
    check: fn() -> Outcome,
}

const SELECTORS: &[SelectorProbe] = &[
    SelectorProbe {
        uses: &["descendant"],
        feature: "Descendant combinator (A B)",
        check: || selects("#w #c", "c"),
    },
    SelectorProbe {
        uses: &[">"],
        feature: "Child combinator (>)",
        check: || selects("#t > #c", "c"),
    },
    SelectorProbe {
        uses: &["+"],
        feature: "Next-sibling combinator (+)",
        check: || selects("#c + div", "d"),
    },
    SelectorProbe {
        uses: &["~"],
        feature: "Subsequent-sibling combinator (~)",
        check: || selects("#s ~ #d", "d"),
    },
    SelectorProbe {
        uses: &["[attr]"],
        feature: "Attribute presence ([attr])",
        check: || selects("#t > [lang]", "c"),
    },
    SelectorProbe {
        uses: &["[=]"],
        feature: "Attribute equals ([attr=value])",
        check: || selects("[id=\"c\"]", "c"),
    },
    SelectorProbe {
        uses: &["[~=]"],
        feature: "Attribute word match ([attr~=value])",
        check: || selects("[class~=\"y\"]", "c"),
    },
    SelectorProbe {
        uses: &["[|=]"],
        feature: "Attribute language prefix ([attr|=value])",
        check: || selects("[lang|=\"en\"]", "c"),
    },
    SelectorProbe {
        uses: &["[^=]"],
        feature: "Attribute prefix ([attr^=value])",
        check: || selects("#t > [id^=\"c\"]", "c"),
    },
    SelectorProbe {
        uses: &["[$=]"],
        feature: "Attribute suffix ([attr$=value])",
        check: || selects("#t > [id$=\"d\"]", "d"),
    },
    SelectorProbe {
        uses: &["[*=]"],
        feature: "Attribute substring ([attr*=value])",
        check: || selects("#t > [class*=\"x y\"]", "c"),
    },
    SelectorProbe {
        uses: &["[ i]"],
        feature: "Case-insensitive attribute matching ([attr=value i])",
        check: || selects("[lang=\"EN-us\" i]", "c"),
    },
    SelectorProbe {
        uses: &["first-child"],
        feature: ":first-child",
        check: || selects("#p > div:first-child", "t"),
    },
    SelectorProbe {
        uses: &["last-child"],
        feature: ":last-child",
        check: || selects("#t > div:last-child", "d"),
    },
    SelectorProbe {
        uses: &["only-child"],
        feature: ":only-child",
        check: || selects("#w > :only-child", "p"),
    },
    SelectorProbe {
        uses: &["nth-child"],
        feature: ":nth-child()",
        check: || selects("#p > :nth-child(2)", "n"),
    },
    SelectorProbe {
        uses: &["nth-last-child"],
        feature: ":nth-last-child()",
        check: || selects("#p > :nth-last-child(2)", "m"),
    },
    SelectorProbe {
        uses: &["first-of-type"],
        feature: ":first-of-type",
        check: || selects("#t > div:first-of-type", "c"),
    },
    SelectorProbe {
        uses: &["last-of-type"],
        feature: ":last-of-type",
        check: || selects("#t > div:last-of-type", "d"),
    },
    SelectorProbe {
        uses: &["nth-of-type"],
        feature: ":nth-of-type()",
        check: || selects("#t > div:nth-of-type(2)", "d"),
    },
    SelectorProbe {
        uses: &["nth-last-of-type"],
        feature: ":nth-last-of-type()",
        check: || selects("#t > div:nth-last-of-type(2)", "c"),
    },
    SelectorProbe {
        uses: &["only-of-type"],
        feature: ":only-of-type",
        check: || selects("#w > div:only-of-type", "p"),
    },
    SelectorProbe {
        uses: &["not"],
        feature: ":not()",
        check: || selects("#t > div:not(#c)", "d"),
    },
    SelectorProbe {
        uses: &["is", "matches", "-webkit-any", "-moz-any"],
        feature: ":is()",
        check: || selects("#t > :is(#c)", "c"),
    },
    SelectorProbe {
        uses: &["where"],
        feature: ":where()",
        check: || selects(":where(#t) > #c", "c"),
    },
    SelectorProbe {
        uses: &["has"],
        feature: ":has()",
        check: || selects("#p > div:has(> #c)", "t"),
    },
    SelectorProbe {
        uses: &["empty"],
        feature: ":empty",
        check: || {
            fixture("#e:empty{height:12px}", |page| {
                page.rect("e")
                    .is_some_and(|e| (e.height - 12.0).abs() < 0.5)
            })
        },
    },
    SelectorProbe {
        uses: &["root"],
        feature: ":root",
        check: || selects(":root #c", "c"),
    },
    SelectorProbe {
        uses: &["link", "any-link"],
        feature: ":link / :any-link",
        check: || selects("#m > :link", "a"),
    },
    SelectorProbe {
        uses: &["lang"],
        feature: ":lang()",
        check: || selects("#c:lang(en)", "c"),
    },
    SelectorProbe {
        uses: &["dir"],
        feature: ":dir()",
        check: || selects("#c:dir(ltr)", "c"),
    },
    SelectorProbe {
        uses: &["before", "after"],
        feature: "::before / ::after generated content",
        check: || {
            fixture("#c::before{content:\"QZXW\"}", |page| {
                page.paints_text("QZXW")
            })
        },
    },
    SelectorProbe {
        uses: &["first-letter"],
        feature: "::first-letter",
        check: || differs("#c::first-letter{color:#fa0000}", ""),
    },
    SelectorProbe {
        uses: &["first-line"],
        feature: "::first-line",
        check: || differs("#c::first-line{color:#fa0000}", ""),
    },
    SelectorProbe {
        uses: &["marker"],
        feature: "::marker",
        check: || {
            differs(
                "#t{display:list-item}#t::marker{color:#fa0000}",
                "#t{display:list-item}",
            )
        },
    },
];

/// Dynamic pseudo-classes that the static compatibility audit cannot exercise
/// because it does not synthesize pointer or focus events.
const INTERACTIVE_PSEUDO_CLASSES: &[&str] = &[
    "hover",
    "active",
    "focus",
    "focus-visible",
    "focus-within",
    "visited",
    "target",
    "target-within",
];

/// One declaration and where it came from.
struct Decl {
    property: String,
    value: String,
    location: String,
}

/// Collects declarations and selectors from every stylesheet and `style`
/// attribute, and reports parse errors.
fn collect(
    page: &PageSources<'_>,
    map: &PageMap<'_>,
    report: &mut AuditReport,
) -> (Vec<Decl>, Usage, Usage) {
    let mut declarations = Vec::new();
    let mut at_rules = Usage::default();
    let mut selectors = Usage::default();
    let mut errors = Usage::default();

    let mut push_declarations = |list: &[Declaration], text: &str, label: &str| {
        for declaration in list {
            let property = declaration.property.clone();
            let line = find_property(text, &property).map(|offset| line_at(text, offset));
            declarations.push(Decl {
                value: declaration.value.to_ascii_lowercase(),
                location: line.map_or_else(|| label.to_owned(), |line| format!("{label}:{line}")),
                property,
            });
        }
    };

    for (index, text) in page.stylesheets.iter().enumerate() {
        let label = map
            .sheet_labels
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("stylesheet #{}", index + 1));
        let sheet = std::panic::catch_unwind(|| css::parse_stylesheet_with_errors(text));
        let Ok((sheet, parse_errors)) = sheet else {
            report.findings.push(Finding::new(
                Area::CssParsing,
                format!("{label} crashed the CSS parser"),
                "None of this stylesheet's rules apply.",
                Evidence::Probed,
            ));
            continue;
        };
        for error in &parse_errors {
            errors.add(&error.message, || {
                format!("{label}:{}", line_at(text, error.offset))
            });
        }
        let mut rules: Vec<&css::Rule> = sheet.rules.iter().collect();
        for at_rule in &sheet.at_rules {
            let offset = text.find(&format!("@{}", at_rule.name)).unwrap_or(0);
            at_rules.add(&at_rule.name, || {
                format!("{label}:{}", line_at(text, offset))
            });
            rules.extend(&at_rule.rules);
        }
        for rule in rules {
            let offset = text.find(&rule.selector.0).unwrap_or(0);
            for feature in selector_features(&rule.selector.0) {
                selectors.add(&feature, || format!("{label}:{}", line_at(text, offset)));
            }
            push_declarations(&rule.declarations, text, &label);
        }
    }
    for (style, line) in &map.inline_styles {
        let sheet = css::parse_stylesheet(&format!("*{{{style}}}"));
        if let Some(rule) = sheet.rules.first() {
            let label = format!("HTML line {line} (style attribute)");
            for declaration in &rule.declarations {
                declarations.push(Decl {
                    property: declaration.property.clone(),
                    value: declaration.value.to_ascii_lowercase(),
                    location: label.clone(),
                });
            }
        }
    }
    for (message, used) in errors.iter() {
        report.findings.push(
            Finding::new(
                Area::CssParsing,
                message.clone(),
                "The CSS parser rejected this, so the affected rule or declaration is dropped.",
                Evidence::Probed,
            )
            .at(used),
        );
    }
    if map.sheet_slots > page.stylesheets.len() {
        report.findings.push(Finding::new(
            Area::CssParsing,
            "Stylesheets failed to load",
            format!(
                "The page references {} stylesheets but only {} loaded.",
                map.sheet_slots,
                page.stylesheets.len()
            ),
            Evidence::Observed,
        ));
    }
    (declarations, at_rules, selectors)
}

/// Byte offset of the first `property:` in `text` that is not the tail of a
/// longer name (so `content` does not match `justify-content:`).
fn find_property(text: &str, property: &str) -> Option<usize> {
    text.match_indices(property)
        .find(|&(index, _)| {
            let before = text[..index].chars().next_back();
            let after = text[index + property.len()..].trim_start();
            !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                && after.starts_with(':')
        })
        .map(|(index, _)| index)
}

/// Splits a selector into the features the probes know about: pseudo names
/// (without colons), combinators, and attribute-selector operators.
fn selector_features(selector: &str) -> Vec<String> {
    let mut features = Vec::new();
    let chars: Vec<char> = selector.chars().collect();
    let mut i = 0;
    let mut depth = 0;
    let mut last_was_space = false;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            '"' | '\'' => {
                i += 1;
                while i < chars.len() && chars[i] != c {
                    i += 1;
                }
            }
            '[' => {
                let end = chars[i..]
                    .iter()
                    .position(|&c| c == ']')
                    .map_or(chars.len(), |p| i + p);
                let inner: String = chars[i + 1..end].iter().collect();
                let operator = ["~=", "|=", "^=", "$=", "*=", "="]
                    .into_iter()
                    .find(|op| inner.contains(op));
                features.push(operator.map_or_else(|| "[attr]".to_owned(), |op| format!("[{op}]")));
                let trimmed = inner.trim_end().to_ascii_lowercase();
                if operator.is_some() && (trimmed.ends_with(" i") || trimmed.ends_with(" s")) {
                    features.push("[ i]".to_owned());
                }
                i = end;
            }
            ':' => {
                let start = if chars.get(i + 1) == Some(&':') {
                    i + 2
                } else {
                    i + 1
                };
                let end = chars[start..]
                    .iter()
                    .position(|c| !(c.is_ascii_alphanumeric() || *c == '-'))
                    .map_or(chars.len(), |p| start + p);
                features.push(
                    chars[start..end]
                        .iter()
                        .collect::<String>()
                        .to_ascii_lowercase(),
                );
                i = end;
                last_was_space = false;
                continue;
            }
            '>' | '+' | '~' if depth == 0 => features.push(c.to_string()),
            c if c.is_whitespace() && depth == 0 => {
                last_was_space = true;
                i += 1;
                continue;
            }
            _ => {
                if last_was_space
                    && !features
                        .last()
                        .is_some_and(|f| matches!(f.as_str(), ">" | "+" | "~"))
                    && i > 0
                {
                    let before = chars[..i].iter().rev().find(|c| !c.is_whitespace());
                    if before.is_some_and(|b| !matches!(b, '>' | '+' | '~' | ',')) {
                        features.push("descendant".to_owned());
                    }
                }
            }
        }
        last_was_space = false;
        i += 1;
    }
    features
}

pub(crate) fn audit<E: PageScriptEngine>(
    auditor: &Auditor<E>,
    page: &PageSources<'_>,
    map: &PageMap<'_>,
    report: &mut AuditReport,
) {
    let (declarations, at_rules, selectors) = collect(page, map, report);
    audit_properties(auditor, &declarations, report);
    audit_values(auditor, &declarations, report);
    audit_at_rules(auditor, &at_rules, report);
    audit_selectors(auditor, &selectors, report);
}

fn audit_properties<E: PageScriptEngine>(
    auditor: &Auditor<E>,
    declarations: &[Decl],
    report: &mut AuditReport,
) {
    let mut usage = Usage::default();
    for declaration in declarations {
        // Custom properties are storage (probed through var()); `content`
        // only applies to pseudo-elements (probed through ::before).
        if declaration.property.starts_with("--") || declaration.property == "content" {
            continue;
        }
        let value = declaration.value.trim_end_matches("!important").trim();
        usage
            .add(&declaration.property, || declaration.location.clone())
            .remember(value, 4);
    }
    let properties: Vec<_> = usage
        .iter()
        .filter(|(property, _)| !UNOBSERVABLE.contains(&property.as_str()))
        .collect();
    report.features_checked += usage.len();

    // Each property needs up to a few hundred renders when it does nothing,
    // so uncached properties are probed on several threads. Workers only
    // render; the cache is read and written on this thread.
    let key = |property: &str| format!("css-property:{property}");
    let pending: Vec<usize> = (0..properties.len())
        .filter(|&index| auditor.known(&key(properties[index].0)).is_none())
        .collect();
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism()
        .map_or(2, usize::from)
        .clamp(1, 8)
        .min(pending.len().max(1));
    let probed: Vec<(usize, bool)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut results = Vec::new();
                    while let Some(&index) = pending.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let (property, used) = properties[index];
                        results.push((index, property_has_effect(property, &used.values)));
                    }
                    results
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap_or_default())
            .collect()
    });
    for (index, works) in probed {
        auditor.remember(key(properties[index].0), works.into());
    }
    let works = (0..properties.len()).map(|index| {
        (
            index,
            auditor.known(&key(properties[index].0)) != Some(Outcome::Broken),
        )
    });
    for (index, works) in works {
        if works {
            continue;
        }
        let (property, used) = properties[index];
        let vendor = property.starts_with('-');
        let detail = if vendor {
            "Vendor-prefixed property with no effect in this engine."
        } else {
            "Setting this property changes nothing the engine renders, in any of the tested \
             layouts and values."
        };
        report.findings.push(
            Finding::new(
                Area::CssProperty,
                property.clone(),
                detail,
                Evidence::Probed,
            )
            .at(used),
        );
    }
    for (property, used) in usage
        .iter()
        .filter(|(p, _)| UNOBSERVABLE.contains(&p.as_str()))
    {
        report.unverified_push(
            property,
            used.location.as_deref(),
            "interactive or print-only CSS property",
        );
    }
}

/// Whether setting `property` to any of the page's `values` or a generic
/// candidate changes the fixture's rendering in any context.
fn property_has_effect(property: &str, values: &[String]) -> bool {
    let try_values = |context: &str, values: Vec<&str>| {
        let Some(base) = render_fixture(context) else {
            return false;
        };
        values.into_iter().any(|value| {
            render_fixture(&format!("{context}#t{{{property}:{value}}}"))
                .is_none_or(|probe| probe.list != base.list)
        })
    };
    let own = || values.iter().map(String::as_str);
    if try_values(
        CONTEXTS[0],
        own().chain(CANDIDATES.iter().copied()).collect(),
    ) {
        return true;
    }
    CONTEXTS[1..].iter().any(|context| {
        try_values(
            context,
            own().chain(CONTEXT_CANDIDATES.iter().copied()).collect(),
        )
    })
}

fn audit_values<E: PageScriptEngine>(
    auditor: &Auditor<E>,
    declarations: &[Decl],
    report: &mut AuditReport,
) {
    for feature in VALUE_FEATURES {
        let mut usage = Usage::default();
        for declaration in declarations {
            if (feature.detect)(&declaration.property, &declaration.value) {
                usage.add(feature.name, || declaration.location.clone());
            }
        }
        let Some((_, used)) = usage.iter().next() else {
            continue;
        };
        report.features_checked += 1;
        record(
            auditor,
            report,
            Area::CssValue,
            feature.name,
            feature.check,
            used,
        );
    }
}

/// Runs a cached probe and records a finding when it fails.
fn record<E: PageScriptEngine>(
    auditor: &Auditor<E>,
    report: &mut AuditReport,
    area: Area,
    feature: &str,
    check: fn() -> Outcome,
    used: &super::Use,
) {
    match auditor.verdict(&format!("css:{feature}"), check) {
        Outcome::Works => {}
        Outcome::Inconclusive => report.unverified_push(
            feature,
            used.location.as_deref(),
            "the probe's reference rendering did not work",
        ),
        Outcome::Broken => report.findings.push(
            Finding::new(
                area,
                feature,
                "Not implemented: a probe render does not match the expected result.",
                Evidence::Probed,
            )
            .at(used),
        ),
    }
}

fn audit_at_rules<E: PageScriptEngine>(
    auditor: &Auditor<E>,
    at_rules: &Usage,
    report: &mut AuditReport,
) {
    for (name, used) in at_rules.iter() {
        if IGNORED_AT_RULES.contains(&name.as_str()) {
            continue;
        }
        report.features_checked += 1;
        if let Some(probe) = AT_RULES
            .iter()
            .find(|probe| probe.names.contains(&name.as_str()))
        {
            record(
                auditor,
                report,
                Area::CssAtRule,
                probe.feature,
                probe.check,
                used,
            );
            continue;
        }
        match name.as_str() {
            "font-face" => {
                let works = auditor.cached("css-property:font-family", || {
                    property_has_effect("font-family", &["monospace".to_owned()])
                });
                if !works {
                    report.findings.push(
                        Finding::new(
                            Area::CssAtRule,
                            "@font-face web fonts",
                            "font-family has no effect in this engine, so downloaded fonts are never used.",
                            Evidence::Probed,
                        )
                        .at(used),
                    );
                }
            }
            "import" => report.findings.push(
                Finding::new(
                    Area::CssAtRule,
                    "@import",
                    "Imported stylesheets are not fetched (documented on browser::Page), so their rules never apply.",
                    Evidence::Documented,
                )
                .at(used),
            ),
            "namespace" => report.findings.push(
                Finding::new(
                    Area::CssAtRule,
                    "@namespace",
                    "Not supported; namespaced selectors match nothing (documented in the css crate).",
                    Evidence::Documented,
                )
                .at(used),
            ),
            other => {
                report.unverified_push(&format!("@{other}"), used.location.as_deref(), "no static probe for this at-rule");
            }
        }
    }
}

fn audit_selectors<E: PageScriptEngine>(
    auditor: &Auditor<E>,
    selectors: &Usage,
    report: &mut AuditReport,
) {
    for probe in SELECTORS {
        let used: Vec<_> = selectors
            .iter()
            .filter(|(name, _)| probe.uses.contains(&name.as_str()))
            .collect();
        let Some((_, first)) = used.first() else {
            continue;
        };
        report.features_checked += 1;
        let before = report.findings.len();
        record(
            auditor,
            report,
            Area::CssSelector,
            probe.feature,
            probe.check,
            first,
        );
        if let Some(finding) = report.findings.get_mut(before) {
            finding.occurrences = used.iter().map(|(_, entry)| entry.count).sum();
        }
    }
    for (name, used) in selectors.iter() {
        if INTERACTIVE_PSEUDO_CLASSES.contains(&name.as_str()) {
            report.features_checked += 1;
            report.findings.push(
                Finding::new(
                    Area::CssSelector,
                    format!(":{name}"),
                    "The compatibility probe is static and does not synthesize pointer or focus \
                     events; runtime interaction behavior is not verified here.",
                    Evidence::Documented,
                )
                .at(used),
            );
        } else if !SELECTORS
            .iter()
            .any(|probe| probe.uses.contains(&name.as_str()))
        {
            report.unverified_push(
                &format!(":{name}"),
                used.location.as_deref(),
                "no static probe for this selector",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_features_find_pseudos_combinators_and_attributes() {
        assert_eq!(
            selector_features("ul > li:nth-child(2n+1) a:hover::before"),
            [">", "nth-child", "descendant", "hover", "before"]
        );
        assert_eq!(
            selector_features("a[href^=\"http\" i] + .x ~ [hidden]"),
            ["[^=]", "[ i]", "+", "~", "[attr]"]
        );
        assert_eq!(selector_features("#a .b"), ["descendant"]);
    }

    #[test]
    fn property_locations_match_whole_names() {
        let css = ".a { justify-content: center }\n.b::after { content: 'x' }";
        assert_eq!(
            find_property(css, "content").map(|o| line_at(css, o)),
            Some(2)
        );
        assert_eq!(
            find_property(css, "justify-content").map(|o| line_at(css, o)),
            Some(1)
        );
    }

    #[test]
    fn value_detectors_respect_function_and_unit_boundaries() {
        assert!(calls("calc(1px + 2px)", "calc"));
        assert!(!calls("minmax(1fr, 2fr)", "min"));
        assert!(calls("min(1px, 2px)", "min"));
        assert!(has_unit("2rem", "rem"));
        assert!(!has_unit("2rem", "em"));
        assert!(has_unit("1.5em", "em"));
        assert!(!has_unit("inherit", "in"));
        assert!(has_hex("#fff", &[3, 4]));
        assert!(!has_hex("#ffffff", &[3, 4]));
    }
}
