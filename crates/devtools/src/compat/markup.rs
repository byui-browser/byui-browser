//! The page's HTML structure: labels for its scripts and stylesheets, and
//! the audit of HTML elements and attributes that need engine behavior.

use html::{HTMLDocument, NodeID, NodeKind};

use super::render::{Rendered, render};
use super::{Area, AuditReport, Auditor, Evidence, Finding, PageScriptEngine, Usage};

/// One `<script>` element, in the order `browser::Page::scripts` uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptSlot {
    /// A short name: the `src` file name or `inline script #n`.
    pub label: String,
    /// The `src` attribute, if any.
    pub src: Option<String>,
    /// The trimmed, lowercased `type` attribute, if any.
    pub kind: Option<String>,
}

impl ScriptSlot {
    /// Whether the `type` names a classic JavaScript script.
    pub fn is_classic(&self) -> bool {
        self.kind.as_deref().is_none_or(|kind| {
            kind.is_empty()
                || matches!(
                    kind.split(';').next().unwrap_or_default().trim(),
                    "text/javascript"
                        | "application/javascript"
                        | "application/ecmascript"
                        | "application/x-ecmascript"
                        | "application/x-javascript"
                        | "text/ecmascript"
                        | "text/jscript"
                        | "text/livescript"
                        | "text/x-ecmascript"
                        | "text/x-javascript"
                )
                || kind.starts_with("text/javascript1.")
        })
    }
}

/// What the audit knows about the page's markup.
pub(crate) struct PageMap {
    pub document: HTMLDocument,
    /// `<script>` elements in document order.
    pub scripts: Vec<ScriptSlot>,
    /// A label per entry of `PageSources::stylesheets`.
    pub sheet_labels: Vec<String>,
    /// How many stylesheets the markup references; more than
    /// `sheet_labels.len()` means some failed to load.
    pub sheet_slots: usize,
    /// `style` attribute values found in the document.
    pub inline_styles: Vec<String>,
}

/// The last path segment of a URL, without query or fragment.
fn file_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path);
    if name.is_empty() {
        url.to_owned()
    } else {
        name.to_owned()
    }
}

/// Elements in arena order, which for parsed documents is document order and
/// matches how the browser's loader collects scripts and stylesheets.
pub(crate) fn elements(
    document: &HTMLDocument,
) -> impl Iterator<Item = (NodeID, &html::ElementData)> {
    (0..)
        .map(NodeID::new)
        .map_while(move |id| document.node(id).map(|node| (id, node)))
        .filter_map(|(id, node)| match node.get_kind() {
            NodeKind::Element(element) => Some((id, element)),
            _ => None,
        })
}

pub(crate) fn attribute<'e>(element: &'e html::ElementData, name: &str) -> Option<&'e str> {
    element
        .get_attributes()
        .iter()
        .find(|attribute| attribute.get_name().eq_ignore_ascii_case(name))
        .map(|attribute| attribute.get_value().as_str())
}

impl PageMap {
    pub fn new(page: &super::PageSources<'_>) -> Self {
        let document = html::parse_raw_html(page.html.to_owned());
        let mut scripts = Vec::new();
        let mut sheets = Vec::new();
        let mut inline_styles = Vec::new();
        let mut inline_scripts = 0;
        let mut style_elements = 0;
        for (_, element) in elements(&document) {
            if let Some(style) = attribute(element, "style")
                && !style.trim().is_empty()
            {
                inline_styles.push(style.to_owned());
            }
            match element.get_name().as_str() {
                "script" => {
                    let src = attribute(element, "src").map(str::to_owned);
                    let label = match &src {
                        Some(src) => file_name(src),
                        None => {
                            inline_scripts += 1;
                            format!("inline script #{inline_scripts}")
                        }
                    };
                    scripts.push(ScriptSlot {
                        label,
                        src,
                        kind: attribute(element, "type")
                            .map(|kind| kind.trim().to_ascii_lowercase()),
                    });
                }
                "style" => {
                    style_elements += 1;
                    sheets.push(format!("<style> #{style_elements}"));
                }
                "link" => {
                    let stylesheet = attribute(element, "rel").is_some_and(|rel| {
                        rel.split_ascii_whitespace()
                            .any(|token| token.eq_ignore_ascii_case("stylesheet"))
                    });
                    if let (true, Some(href)) = (stylesheet, attribute(element, "href")) {
                        sheets.push(file_name(href));
                    }
                }
                _ => {}
            }
        }
        let sheet_slots = sheets.len();
        // Failed stylesheet loads are dropped, so labels only line up when
        // every referenced stylesheet arrived.
        let sheet_labels = if sheets.len() == page.stylesheets.len() {
            sheets
        } else {
            (1..=page.stylesheets.len())
                .map(|index| format!("stylesheet #{index}"))
                .collect()
        };
        Self {
            document,
            scripts,
            sheet_labels,
            sheet_slots,
            inline_styles,
        }
    }

    /// The label for the script at `index` of `PageSources::scripts`.
    pub fn script_label(&self, index: usize) -> String {
        self.scripts.get(index).map_or_else(
            || format!("script #{}", index + 1),
            |slot| slot.label.clone(),
        )
    }
}

/// An HTML feature the engine must implement for the page to look right.
struct ElementProbe {
    /// Element names (or `[attr]` for attributes) that use the feature.
    uses: &'static [&'static str],
    /// The feature name shown in the report.
    feature: &'static str,
    /// What a working engine does.
    expected: &'static str,
    /// Returns whether the engine behaves as expected.
    check: fn() -> bool,
}

fn body(markup: &str) -> Option<Rendered> {
    render(
        &format!("<!DOCTYPE html><html><body>{markup}</body></html>"),
        "",
    )
}

/// Whether the element `#x` in `markup` gets a `width` × `height` box.
fn sized(markup: &str, width: f32, height: f32) -> bool {
    body(markup)
        .and_then(|page| page.rect("x"))
        .is_some_and(|rect| (rect.width - width).abs() < 0.5 && (rect.height - height).abs() < 0.5)
}

/// Whether `#x` in `markup` generates a box with a non-zero area.
fn visible_box(markup: &str) -> bool {
    body(markup)
        .and_then(|page| page.rect("x"))
        .is_some_and(|rect| rect.width > 0.0 && rect.height > 0.0)
}

/// Whether `markup` paints `needle` as text.
fn paints(markup: &str, needle: &str) -> bool {
    body(markup).is_some_and(|page| page.paints_text(needle))
}

/// Whether wrapping text in `<tag>` paints differently from a plain `<span>`.
fn styled(tag: &str, attributes: &str) -> bool {
    let tagged = body(&format!(
        "<p>plain <{tag}{attributes}>styled</{tag}> text</p>"
    ));
    let plain = body("<p>plain <span>styled</span> text</p>");
    matches!((tagged, plain), (Some(tagged), Some(plain)) if tagged.list != plain.list)
}

const ELEMENT_PROBES: &[ElementProbe] = &[
    ElementProbe {
        uses: &["img"],
        feature: "<img> images",
        expected: "Sized from width/height and painted as a replaced element",
        check: || {
            sized(
                "<img id=\"x\" width=\"40\" height=\"30\" alt=\"\">",
                40.0,
                30.0,
            )
        },
    },
    ElementProbe {
        uses: &["picture"],
        feature: "<picture>",
        expected: "Its <img> is sized and painted as a replaced element",
        check: || {
            sized(
                "<picture><img id=\"x\" width=\"40\" height=\"30\" alt=\"\"></picture>",
                40.0,
                30.0,
            )
        },
    },
    ElementProbe {
        uses: &["svg"],
        feature: "Inline <svg>",
        expected: "Sized from width/height and its shapes painted",
        check: || {
            sized(
                "<svg id=\"x\" width=\"40\" height=\"30\"><rect width=\"40\" height=\"30\"/></svg>",
                40.0,
                30.0,
            )
        },
    },
    ElementProbe {
        uses: &["canvas"],
        feature: "<canvas>",
        expected: "A 300×150 drawing surface by default",
        check: || sized("<canvas id=\"x\"></canvas>", 300.0, 150.0),
    },
    ElementProbe {
        uses: &["video"],
        feature: "<video>",
        expected: "A 300×150 media box by default",
        check: || sized("<video id=\"x\"></video>", 300.0, 150.0),
    },
    ElementProbe {
        uses: &["audio"],
        feature: "<audio controls>",
        expected: "Visible playback controls",
        check: || visible_box("<audio id=\"x\" controls></audio>"),
    },
    ElementProbe {
        uses: &["iframe"],
        feature: "<iframe>",
        expected: "A nested browsing context, 300×150 by default",
        check: || {
            body("<iframe id=\"x\"></iframe>")
                .and_then(|page| page.rect("x"))
                .is_some_and(|rect| rect.width >= 300.0 && rect.height >= 150.0)
        },
    },
    ElementProbe {
        uses: &["embed", "object"],
        feature: "<embed> / <object>",
        expected: "Sized from width/height as an embedded resource",
        check: || sized("<embed id=\"x\" width=\"40\" height=\"30\">", 40.0, 30.0),
    },
    ElementProbe {
        uses: &["input"],
        feature: "<input> form controls",
        expected: "A visible text field even when empty",
        check: || visible_box("<input id=\"x\">"),
    },
    ElementProbe {
        uses: &["textarea"],
        feature: "<textarea>",
        expected: "A visible multi-line text field",
        check: || visible_box("<textarea id=\"x\"></textarea>"),
    },
    ElementProbe {
        uses: &["select"],
        feature: "<select>",
        expected: "A drop-down showing the selected option",
        check: || visible_box("<select id=\"x\"><option>One</option></select>"),
    },
    ElementProbe {
        uses: &["button"],
        feature: "<button> appearance",
        expected: "A visible button box even without text",
        check: || visible_box("<button id=\"x\"></button>"),
    },
    ElementProbe {
        uses: &["progress", "meter"],
        feature: "<progress> / <meter>",
        expected: "A visible gauge",
        check: || visible_box("<progress id=\"x\" value=\"1\" max=\"2\"></progress>"),
    },
    ElementProbe {
        uses: &["hr"],
        feature: "<hr> rule",
        expected: "A visible horizontal line",
        check: || {
            let with = body("<p>a</p><hr><p>b</p>");
            let without = body("<p>a</p><div></div><p>b</p>");
            matches!((with, without), (Some(with), Some(without)) if with.list != without.list)
        },
    },
    ElementProbe {
        uses: &["template"],
        feature: "<template>",
        expected: "Its content is inert and not rendered",
        check: || !paints("<template><p>templated</p></template>", "templated"),
    },
    ElementProbe {
        uses: &["details"],
        feature: "<details> / <summary>",
        expected: "Content stays hidden until the disclosure is opened",
        check: || {
            let markup = "<details><summary>summary</summary><p>disclosed</p></details>";
            paints(markup, "summary") && !paints(markup, "disclosed")
        },
    },
    ElementProbe {
        uses: &["dialog"],
        feature: "<dialog>",
        expected: "Hidden until opened",
        check: || !paints("<dialog><p>dialogtext</p></dialog>", "dialogtext"),
    },
    ElementProbe {
        uses: &["noscript"],
        feature: "<noscript>",
        expected: "Hidden, because scripting is enabled",
        check: || !paints("<noscript><p>fallback</p></noscript>", "fallback"),
    },
    ElementProbe {
        uses: &["[hidden]"],
        feature: "hidden attribute",
        expected: "Elements with `hidden` are not rendered",
        check: || !paints("<div hidden>concealed</div>", "concealed"),
    },
    ElementProbe {
        uses: &["table", "tr", "td", "th"],
        feature: "Table layout",
        expected: "Cells in a row sit side by side",
        check: || {
            body("<table><tr><td id=\"x\">AA</td><td id=\"y\">BB</td></tr></table>").is_some_and(
                |page| match (page.rect("x"), page.rect("y")) {
                    (Some(a), Some(b)) => (a.y - b.y).abs() < 0.5 && b.x > a.x,
                    _ => false,
                },
            )
        },
    },
    ElementProbe {
        uses: &["li"],
        feature: "List markers",
        expected: "Bullets or numbers before list items",
        check: || {
            let list = body("<ul><li>item</li></ul>");
            let blocks = body("<ul><li style=\"display:block\">item</li></ul>");
            matches!((list, blocks), (Some(list), Some(blocks)) if list.list != blocks.list)
        },
    },
    ElementProbe {
        uses: &["pre"],
        feature: "<pre> whitespace",
        expected: "Spaces and line breaks are preserved",
        check: || paints("<pre>a   b</pre>", "a   b"),
    },
    ElementProbe {
        uses: &["sup", "sub"],
        feature: "<sup> / <sub>",
        expected: "Text raised or lowered from the baseline",
        check: || {
            body("<p>base<sup>raised</sup></p>").is_some_and(|page| {
                match (page.text_position("base"), page.text_position("raised")) {
                    (Some((_, base)), Some((_, raised))) => raised < base,
                    _ => false,
                }
            })
        },
    },
    ElementProbe {
        uses: &["[dir=rtl]"],
        feature: "dir=\"rtl\"",
        expected: "Right-to-left text starts at the right edge",
        check: || {
            body("<p dir=\"rtl\">rtl</p>")
                .and_then(|page| page.text_position("rtl"))
                .is_some_and(|(x, _)| x > 400.0)
        },
    },
    ElementProbe {
        uses: &["a"],
        feature: "Link styling",
        expected: "colored, underlined link text; renders like a plain <span>",
        check: || styled("a", " href=\"#\""),
    },
    ElementProbe {
        uses: &["b", "strong"],
        feature: "<b> / <strong>",
        expected: "bold text; renders like a plain <span>",
        check: || styled("b", ""),
    },
    ElementProbe {
        uses: &["i", "em", "cite", "var", "dfn", "address"],
        feature: "<i> / <em>",
        expected: "italic text; renders like a plain <span>",
        check: || styled("i", ""),
    },
    ElementProbe {
        uses: &["u", "ins"],
        feature: "<u> / <ins>",
        expected: "underlined text; renders like a plain <span>",
        check: || styled("u", ""),
    },
    ElementProbe {
        uses: &["s", "del", "strike"],
        feature: "<s> / <del>",
        expected: "struck-through text; renders like a plain <span>",
        check: || styled("s", ""),
    },
    ElementProbe {
        uses: &["code", "kbd", "samp", "tt"],
        feature: "<code> / <kbd>",
        expected: "monospace text; renders like a plain <span>",
        check: || styled("code", ""),
    },
    ElementProbe {
        uses: &["mark"],
        feature: "<mark>",
        expected: "highlighted text; renders like a plain <span>",
        check: || styled("mark", ""),
    },
    ElementProbe {
        uses: &["small"],
        feature: "<small>",
        expected: "smaller text; renders like a plain <span>",
        check: || styled("small", ""),
    },
    ElementProbe {
        uses: &["q"],
        feature: "<q> quotation marks",
        expected: "quotation marks around the text; renders like a plain <span>",
        check: || styled("q", ""),
    },
];

/// Audits HTML elements and attributes that need engine behavior.
pub(crate) fn audit<E: PageScriptEngine>(
    auditor: &Auditor<E>,
    map: &PageMap,
    report: &mut AuditReport,
) {
    let mut usage = Usage::default();
    for (_, element) in elements(&map.document) {
        let location = || "HTML markup".to_owned();
        usage.add(element.get_name(), location);
        if attribute(element, "hidden").is_some() {
            usage.add("[hidden]", location);
        }
        if attribute(element, "dir").is_some_and(|dir| dir.eq_ignore_ascii_case("rtl")) {
            usage.add("[dir=rtl]", location);
        }
    }
    let entries: Vec<_> = usage.iter().collect();
    for probe in ELEMENT_PROBES {
        let used: Vec<_> = entries
            .iter()
            .filter(|(name, _)| probe.uses.contains(&name.as_str()))
            .collect();
        let Some((_, first)) = used.first() else {
            continue;
        };
        report.features_checked += 1;
        if auditor.cached(&format!("html:{}", probe.feature), probe.check) {
            continue;
        }
        let detail = format!("Not implemented. Expected: {}.", probe.expected);
        let mut finding =
            Finding::new(Area::Html, probe.feature, detail, Evidence::Probed).at(first);
        finding.occurrences = used.iter().map(|(_, entry)| entry.count).sum();
        report.findings.push(finding);
    }
}
