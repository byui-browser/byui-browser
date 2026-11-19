//! Box generation: from the DOM and computed styles to the box tree
//! (CSS Display 3 §2, CSS 2.2 §9.2).

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use css::PseudoElement;

use html::{HTMLDocument, NodeID, NodeKind};

use crate::control::{CHECKBOX_RADIO_SIZE, ControlKind, FormControl, ToggleControl};
use crate::replaced::{
    ImageKey, ImageMetrics, NaturalSize, ReplacedBox, parse_dimension_attribute, view_box_ratio,
};
use crate::style::{ComputedStyle, Dimension, Display, DisplayInside, DisplayOutside};

/// A block container box before layout.
#[derive(Debug, Clone)]
pub(crate) struct BlockBox {
    /// The generating element, or `None` for anonymous boxes and the initial
    /// containing block.
    pub node: Option<NodeID>,
    pub style: Rc<ComputedStyle>,
    /// Whether the box establishes a new block formatting context
    /// (CSS 2.2 §9.4.1), which stops margins collapsing through it.
    pub establishes_bfc: bool,
    /// Whether text painted inside the box is clipped to its padding box.
    /// Set for form controls, whose contents the user agent draws (the
    /// built-in stylesheet gives inputs `overflow: clip`). The general
    /// `overflow` property is not implemented yet.
    pub clips_contents: bool,
    /// Whether the box's line boxes are centered vertically in its content
    /// box when it is taller than they are, as single-line form controls
    /// do. Ordinary blocks start their lines at the top.
    pub centers_lines: bool,
    pub contents: BlockContents,
    /// The form control the box is, which gives `auto` sizes a default.
    pub control: Option<FormControl>,
}

/// A block container holds either only block-level boxes or only inline-level
/// content (CSS 2.2 §9.2.1.1).
#[derive(Debug, Clone)]
pub(crate) enum BlockContents {
    /// Block-level children. Floated and absolutely positioned children
    /// appear here in tree order too; layout takes them out of flow.
    Blocks(Vec<BlockBox>),
    Inlines(Vec<InlineItem>),
    /// The box is a replaced element (`<img>`, inline `<svg>`); its content
    /// is a picture rather than child boxes.
    Replaced(ReplacedBox),
}

/// Inline-level content of an inline formatting context, flattened in tree
/// order. Inline boxes are bracketed by `Start` and `End`.
#[derive(Debug, Clone)]
pub(crate) enum InlineItem {
    Text {
        node: NodeID,
        element: Option<NodeID>,
        text: String,
        style: Rc<ComputedStyle>,
        /// How the run's white space is processed. DOM text is
        /// [`WhiteSpace::Collapse`]; text the user agent draws inside form
        /// controls uses the other modes.
        white_space: WhiteSpace,
    },
    /// The start of an inline box. `start_edge` is `false` on the
    /// continuation after a block-level box split the inline box.
    Start {
        node: NodeID,
        style: Rc<ComputedStyle>,
        start_edge: bool,
    },
    /// The end of the innermost open inline box. `end_edge` is `false` when a
    /// block-level box splits the inline box here.
    End { end_edge: bool },
    /// An atomic inline-level box (an inline-block).
    Atomic(BlockBox),
    /// A forced line break from a `<br>` element.
    ForcedBreak,
    /// A floated or absolutely positioned box. It takes no space in the line;
    /// layout places it out of flow from this point (CSS 2.2 §9.5, §9.6).
    OutOfFlow(BlockBox),
}

/// White-space processing for one text run (a subset of CSS Text 3 §3's
/// `white-space`). The `white-space` property itself is not supported yet:
/// DOM text always collapses, and only form controls pick another mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WhiteSpace {
    /// `normal`: runs of white space collapse to one space, which is a soft
    /// wrap opportunity and is removed at the start and end of a line.
    Collapse,
    /// `pre` without segment breaks: the run is one unbreakable piece with
    /// its spaces kept. Used for single-line control text; input values
    /// arrive with each tab already replaced by one space.
    Pre,
    /// `pre-wrap` without segment breaks: spaces and tabs are kept (a tab is
    /// drawn as one space, since tab stops are not supported), a soft wrap
    /// opportunity follows each run of them, and spaces at the end of a
    /// line hang (they do not count toward fitting and are not painted).
    /// Callers split segment breaks into forced breaks. Used for
    /// `<textarea>` text.
    PreWrap,
}

/// Either kind of child produced while walking a block container's children.
enum Level {
    Block(BlockBox),
    Inline(InlineItem),
}

/// Computed styles for every element, indexed by DOM arena index.
pub(crate) struct StyleMap {
    styles: Vec<Option<Rc<ComputedStyle>>>,
    /// `::before` / `::after` styles, keyed by originating element index.
    pseudo: HashMap<(usize, PseudoElement), Rc<ComputedStyle>>,
    initial: Rc<ComputedStyle>,
}

/// The computed `text-align` of `node`'s nearest element ancestor, for
/// `match-parent`. Ancestors whose own value is `match-parent` defer to
/// theirs in turn; the root falls back to `start`.
fn parent_text_align(
    document: &HTMLDocument,
    styles: &css::ComputedStyles,
    node: NodeID,
) -> crate::style::TextAlign {
    let mut current = document
        .nodes
        .get(node.get())
        .and_then(|node| node.get_parent());
    while let Some(ancestor) = current {
        if let Some(declarations) = styles.declarations_for(ancestor) {
            let value = declarations
                .iter()
                .find(|declaration| declaration.property == "text-align")
                .map(|declaration| declaration.value.as_str());
            let align =
                crate::style::TextAlign::compute(value, || crate::style::TextAlign::MatchParent);
            if align != crate::style::TextAlign::MatchParent {
                return align;
            }
        }
        current = document
            .nodes
            .get(ancestor.get())
            .and_then(|node| node.get_parent());
    }
    crate::style::TextAlign::Start
}

impl StyleMap {
    /// Resolves every element's computed style. The root element's display is
    /// blockified (CSS Display 3 §2.8).
    pub(crate) fn new(
        document: &HTMLDocument,
        styles: &css::ComputedStyles,
        root: Option<NodeID>,
        viewport: crate::Size,
        fonts: &dyn crate::FontMetrics,
    ) -> Self {
        let root_font_size = root
            .and_then(|root| styles.value_for(root, "font-size"))
            .and_then(|value| {
                css::values::compute_font_size(
                    value,
                    css::values::MEDIUM_FONT_SIZE,
                    css::values::MEDIUM_FONT_SIZE,
                )
            })
            .unwrap_or(css::values::MEDIUM_FONT_SIZE);
        let resolved: Vec<Option<Rc<ComputedStyle>>> = (0..document.nodes.len())
            .map(|index| {
                let node = NodeID::new(index);
                let declarations = styles.declarations_for(node)?;
                let parent_text_align = || parent_text_align(document, styles, node);
                let mut style = ComputedStyle::from_declarations(
                    declarations,
                    root_font_size,
                    viewport,
                    parent_text_align,
                );
                if Some(node) == root {
                    style.display = style.display.blockified();
                }
                style.font =
                    fonts.select_font(&style.font_families, style.font_weight, style.font_italic);
                Some(Rc::new(style))
            })
            .collect();
        let mut pseudo = HashMap::new();
        for (index, owner) in resolved.iter().enumerate() {
            let Some(owner) = owner else { continue };
            for which in [PseudoElement::Before, PseudoElement::After] {
                let Some(declarations) = styles.pseudo_declarations_for(NodeID::new(index), which)
                else {
                    continue;
                };
                let style = ComputedStyle::from_declarations(
                    declarations,
                    root_font_size,
                    viewport,
                    || owner.text_align,
                );
                pseudo.insert((index, which), Rc::new(style));
            }
        }
        Self {
            styles: resolved,
            pseudo,
            initial: Rc::new(ComputedStyle::anonymous(None)),
        }
    }

    pub(crate) fn get(&self, node: NodeID) -> Option<&Rc<ComputedStyle>> {
        self.styles.get(node.get())?.as_ref()
    }

    /// The style of `owner`'s `::before` or `::after`, if a rule targets it.
    fn pseudo(&self, owner: NodeID, which: PseudoElement) -> Option<&Rc<ComputedStyle>> {
        self.pseudo.get(&(owner.get(), which))
    }

    /// The style text inherits: its parent element's style, or initial values
    /// for text directly under the document.
    fn for_text_parent(&self, parent: Option<NodeID>) -> Rc<ComputedStyle> {
        parent
            .and_then(|parent| self.get(parent))
            .cloned()
            .unwrap_or_else(|| Rc::clone(&self.initial))
    }

    pub(crate) fn initial(&self) -> &Rc<ComputedStyle> {
        &self.initial
    }
}

/// Returns the document element: the first element child of the document.
pub(crate) fn root_element(document: &HTMLDocument) -> Option<NodeID> {
    document
        .node(document.root)?
        .get_children()
        .iter()
        .copied()
        .find(|child| {
            matches!(
                document.node(*child).map(|node| node.get_kind()),
                Some(NodeKind::Element(_))
            )
        })
}

/// Builds the box tree under the initial containing block.
///
/// The initial containing block is a block container for the document's
/// children. It establishes the root block formatting context. Text or extra
/// elements beside the document element (which a conforming HTML parser does
/// not produce) are boxed as ordinary flow content.
pub(crate) fn build_initial_containing_block(
    document: &HTMLDocument,
    styles: &StyleMap,
    images: &dyn ImageMetrics,
) -> BlockBox {
    let builder = Builder {
        document,
        styles,
        images,
        stats: BuildStats::default(),
    };
    let root = root_element(document);
    if root.is_none() {
        log::trace!("box tree: document has no root element");
    }
    let style = Rc::clone(styles.initial());
    let children = document
        .node(document.root)
        .map(|node| node.get_children().as_slice())
        .unwrap_or_default();
    let block = BlockBox {
        node: None,
        contents: builder.container_contents(None, children, &style),
        style,
        establishes_bfc: true,
        control: None,
        clips_contents: false,
        centers_lines: false,
    };
    let stats = &builder.stats;
    log::debug!(
        "box tree built: root element {}, {} boxes, {} anonymous boxes, {} text runs, {} elements skipped",
        if root.is_some() { "found" } else { "missing" },
        stats.boxes.get() + 1,
        stats.anonymous.get(),
        stats.text_runs.get(),
        stats.skipped.get()
    );
    block
}

/// Counters summarizing one box tree build, for logging.
#[derive(Default)]
struct BuildStats {
    /// Boxes generated by elements (block-level and atomic inline).
    boxes: Cell<usize>,
    /// Anonymous block wrappers created.
    anonymous: Cell<usize>,
    /// Text nodes turned into inline text items.
    text_runs: Cell<usize>,
    /// Elements that generated no box (`display: none` or no computed style).
    skipped: Cell<usize>,
}

impl BuildStats {
    fn bump(counter: &Cell<usize>) {
        counter.set(counter.get() + 1);
    }
}

struct Builder<'a> {
    document: &'a HTMLDocument,
    styles: &'a StyleMap,
    images: &'a dyn ImageMetrics,
    stats: BuildStats,
}

impl Builder<'_> {
    /// The tag name of an element node, or `""` for other nodes.
    fn tag(&self, node: NodeID) -> &str {
        match self.document.node(node).map(|node| node.get_kind()) {
            Some(NodeKind::Element(element)) => element.get_name(),
            _ => "",
        }
    }

    /// Builds the box for an element whose outer display is block or whose
    /// inner display makes it atomic.
    fn block_box(&self, node: NodeID, style: &Rc<ComputedStyle>, is_root: bool) -> BlockBox {
        let flow_root = matches!(
            style.display,
            Display::Box {
                inside: DisplayInside::FlowRoot
                    | DisplayInside::Flex
                    | DisplayInside::Grid
                    | DisplayInside::Table,
                ..
            }
        );
        let children = self
            .document
            .node(node)
            .map(|node| node.get_children().as_slice())
            .unwrap_or_default();
        BuildStats::bump(&self.stats.boxes);
        log::trace!(
            "box for node {node:?} <{}>: display {:?}, bfc {}",
            self.tag(node),
            style.display,
            is_root || flow_root
        );
        let control = self.form_control_sizing(node);
        let contents = match &control {
            // A drop-down shows only its selected option's text, as one
            // synthetic run; its options and groups generate no boxes, and
            // it never shows `::before` / `::after` content.
            Some(FormControl::Select {
                drop_down: Some(text),
                ..
            }) => BlockContents::Inlines(
                (!text.is_empty())
                    .then(|| {
                        BuildStats::bump(&self.stats.text_runs);
                        let style = self.styles.for_text_parent(Some(node));
                        InlineItem::Text {
                            node,
                            element: Some(node),
                            text: style.text_transform.apply(text),
                            style,
                            white_space: WhiteSpace::Collapse,
                        }
                    })
                    .into_iter()
                    .collect(),
            ),
            _ => self.container_contents(Some(node), children, style),
        };
        BlockBox {
            node: Some(node),
            contents,
            style: Rc::clone(style),
            // Floats and absolutely positioned boxes establish a new block
            // formatting context (CSS 2.2 §9.4.1).
            establishes_bfc: is_root || flow_root || style.is_out_of_flow(),
            control,
            clips_contents: false,
            centers_lines: false,
        }
    }

    /// The form control `node` is, which gives its `auto` sizes a default;
    /// `None` for other nodes.
    fn form_control_sizing(&self, node: NodeID) -> Option<FormControl> {
        match self.document.node(node).map(|node| node.get_kind()) {
            Some(NodeKind::Element(element)) => {
                FormControl::for_element(self.document, node, element)
            }
            _ => None,
        }
    }

    /// Boxes a block container's children, wrapping inline-level runs in
    /// anonymous block boxes when block-level siblings exist.
    /// `owner` is the element whose children these are, which may also
    /// generate `::before` / `::after` content around them.
    fn container_contents(
        &self,
        owner: Option<NodeID>,
        children: &[NodeID],
        style: &Rc<ComputedStyle>,
    ) -> BlockContents {
        let mut levels = Vec::new();
        let mut open_inlines = Vec::new();
        if let Some(owner) = owner {
            self.generated(owner, PseudoElement::Before, &mut levels, &open_inlines);
        }
        for child in children {
            self.collect(*child, &mut levels, &mut open_inlines);
        }
        if let Some(owner) = owner {
            self.generated(owner, PseudoElement::After, &mut levels, &open_inlines);
        }
        if !levels.iter().any(|level| matches!(level, Level::Block(_))) {
            return BlockContents::Inlines(
                levels
                    .into_iter()
                    .filter_map(|level| match level {
                        Level::Inline(item) => Some(item),
                        Level::Block(_) => None,
                    })
                    .collect(),
            );
        }
        let mut blocks = Vec::new();
        let mut run = Vec::new();
        let flush = |run: &mut Vec<InlineItem>, blocks: &mut Vec<BlockBox>| {
            let items = std::mem::take(run);
            if items
                .iter()
                .all(|item| is_collapsible_whitespace(item) || is_out_of_flow(item))
            {
                // Out-of-flow boxes between blocks need no anonymous wrapper.
                let count = items.len();
                blocks.extend(items.into_iter().filter_map(|item| match item {
                    InlineItem::OutOfFlow(block) => Some(block),
                    _ => None,
                }));
                // White space that would be collapsed away generates no
                // anonymous inline boxes (CSS 2.2 §9.2.2.1).
                if count > 0 {
                    log::trace!(
                        "dropped whitespace-only inline run of {count} items between blocks"
                    );
                }
                return;
            }
            BuildStats::bump(&self.stats.anonymous);
            log::trace!(
                "anonymous block wrapper created around {} inline items",
                items.len()
            );
            blocks.push(BlockBox {
                node: None,
                style: Rc::new(ComputedStyle::anonymous(Some(style))),
                establishes_bfc: false,
                clips_contents: false,
                centers_lines: false,
                contents: BlockContents::Inlines(items),
                control: None,
            });
        };
        for level in levels {
            match level {
                Level::Inline(item) => run.push(item),
                Level::Block(block) => {
                    flush(&mut run, &mut blocks);
                    blocks.push(block);
                }
            }
        }
        flush(&mut run, &mut blocks);
        BlockContents::Blocks(blocks)
    }

    /// Boxes `owner`'s `::before` or `::after`, if it has a style with
    /// supported `content` (CSS Generated Content 3 §2).
    ///
    /// The generated text is attributed to `owner`, so hit tests and text
    /// search treat it as part of the element. A block-level pseudo-element
    /// becomes an anonymous-style block box holding the text; any other
    /// display other than `none` is boxed as an inline box (inline-block,
    /// flex, and grid pseudo-elements are not supported yet).
    fn generated(
        &self,
        owner: NodeID,
        which: PseudoElement,
        levels: &mut Vec<Level>,
        open_inlines: &[(NodeID, Rc<ComputedStyle>)],
    ) {
        let Some(style) = self.styles.pseudo(owner, which) else {
            return;
        };
        let Some(content) = &style.content else {
            return;
        };
        let text = InlineItem::Text {
            node: owner,
            element: Some(owner),
            text: style.text_transform.apply(content),
            style: Rc::clone(style),
            white_space: WhiteSpace::Collapse,
        };
        if style.is_out_of_flow() && style.display != Display::None {
            log::trace!("out-of-flow {which:?} box for node {owner:?}");
            BuildStats::bump(&self.stats.boxes);
            levels.push(Level::Inline(InlineItem::OutOfFlow(BlockBox {
                node: None,
                style: Rc::clone(style),
                establishes_bfc: true,
                contents: BlockContents::Inlines(vec![text]),
                control: None,
                clips_contents: false,
                centers_lines: false,
            })));
            return;
        }
        match style.display {
            Display::None => {}
            Display::Box {
                outside: DisplayOutside::Block,
                ..
            } => {
                BuildStats::bump(&self.stats.boxes);
                log::trace!("block {which:?} box for node {owner:?}");
                for _ in open_inlines {
                    levels.push(Level::Inline(InlineItem::End { end_edge: false }));
                }
                levels.push(Level::Block(BlockBox {
                    node: None,
                    style: Rc::clone(style),
                    establishes_bfc: false,
                    contents: BlockContents::Inlines(vec![text]),
                    control: None,
                    clips_contents: false,
                    centers_lines: false,
                }));
                for (open, open_style) in open_inlines {
                    levels.push(Level::Inline(InlineItem::Start {
                        node: *open,
                        style: Rc::clone(open_style),
                        start_edge: false,
                    }));
                }
            }
            _ => {
                log::trace!("inline {which:?} box for node {owner:?}");
                levels.push(Level::Inline(InlineItem::Start {
                    node: owner,
                    style: Rc::clone(style),
                    start_edge: true,
                }));
                levels.push(Level::Inline(text));
                levels.push(Level::Inline(InlineItem::End { end_edge: true }));
            }
        }
    }

    fn collect(
        &self,
        node: NodeID,
        levels: &mut Vec<Level>,
        open_inlines: &mut Vec<(NodeID, Rc<ComputedStyle>)>,
    ) {
        let Some(dom_node) = self.document.node(node) else {
            return;
        };
        match dom_node.get_kind() {
            NodeKind::Text(text) => {
                BuildStats::bump(&self.stats.text_runs);
                let style = self.styles.for_text_parent(dom_node.get_parent());
                levels.push(Level::Inline(InlineItem::Text {
                    node,
                    element: dom_node
                        .get_parent()
                        .filter(|parent| self.styles.get(*parent).is_some()),
                    text: style.text_transform.apply(text),
                    style,
                    white_space: WhiteSpace::Collapse,
                }));
            }
            NodeKind::Element(element) => {
                let Some(style) = self.styles.get(node) else {
                    BuildStats::bump(&self.stats.skipped);
                    log::trace!(
                        "skipped node {node:?} <{}>: no computed style",
                        element.get_name()
                    );
                    return;
                };
                let boxed = !matches!(style.display, Display::None | Display::Contents);
                // Form controls with user-agent drawn contents are boxed like
                // replaced elements, so floated or absolutely positioned
                // controls go out of flow below too.
                if boxed
                    && let Some(level) = self
                        .replaced(node, element, style)
                        .or_else(|| self.form_control(node, element, style))
                {
                    match level {
                        Level::Block(block) | Level::Inline(InlineItem::Atomic(block))
                            if style.is_out_of_flow() =>
                        {
                            levels.push(Level::Inline(InlineItem::OutOfFlow(block)));
                        }
                        Level::Block(block) => split_inlines_around(block, levels, open_inlines),
                        inline => levels.push(inline),
                    }
                    return;
                }
                if boxed && style.is_out_of_flow() {
                    log::trace!(
                        "node {node:?} <{}>: out of flow (position {:?}, float {:?})",
                        element.get_name(),
                        style.position,
                        style.float
                    );
                    let block = self.block_box(node, style, false);
                    levels.push(Level::Inline(InlineItem::OutOfFlow(block)));
                    return;
                }
                match style.display {
                    Display::None => {
                        BuildStats::bump(&self.stats.skipped);
                        log::trace!(
                            "skipped node {node:?} <{}>: display none",
                            element.get_name()
                        );
                    }
                    Display::Contents => {
                        log::trace!(
                            "node {node:?} <{}>: display contents, boxing children in its place",
                            element.get_name()
                        );
                        self.generated(node, PseudoElement::Before, levels, open_inlines);
                        for child in dom_node.get_children() {
                            self.collect(*child, levels, open_inlines);
                        }
                        self.generated(node, PseudoElement::After, levels, open_inlines);
                    }
                    Display::Box {
                        outside: DisplayOutside::Inline,
                        inside: DisplayInside::Flow,
                    } => {
                        if element.get_name() == "br" {
                            levels.push(Level::Inline(InlineItem::ForcedBreak));
                            return;
                        }
                        levels.push(Level::Inline(InlineItem::Start {
                            node,
                            style: Rc::clone(style),
                            start_edge: true,
                        }));
                        open_inlines.push((node, Rc::clone(style)));
                        self.generated(node, PseudoElement::Before, levels, open_inlines);
                        for child in dom_node.get_children() {
                            self.collect(*child, levels, open_inlines);
                        }
                        self.generated(node, PseudoElement::After, levels, open_inlines);
                        open_inlines.pop();
                        levels.push(Level::Inline(InlineItem::End { end_edge: true }));
                    }
                    Display::Box {
                        outside: DisplayOutside::Inline,
                        inside:
                            DisplayInside::FlowRoot
                            | DisplayInside::Flex
                            | DisplayInside::Grid
                            | DisplayInside::Table,
                    } => {
                        let atomic = self.block_box(node, style, false);
                        levels.push(Level::Inline(InlineItem::Atomic(atomic)));
                    }
                    Display::Box {
                        outside: DisplayOutside::Block,
                        ..
                    } => {
                        let is_root = dom_node.get_parent() == Some(self.document.root)
                            && root_element(self.document) == Some(node);
                        split_inlines_around(
                            self.block_box(node, style, is_root),
                            levels,
                            open_inlines,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

impl Builder<'_> {
    /// The box for a replaced element (`<img>`, an `<svg>` element that is
    /// not inside another SVG element, or a checkbox or radio `<input>`, which
    /// is sized like a 13x13 image), or `None` for other elements.
    ///
    /// A broken `<img>` (no `src`, or one the embedder has no image for) with
    /// a non-empty `alt` and no `width`/`height` is shown as its `alt` text
    /// instead, like browsers do. Other broken images keep their specified
    /// size and paint nothing.
    fn replaced(
        &self,
        node: NodeID,
        element: &html::ElementData,
        style: &Rc<ComputedStyle>,
    ) -> Option<Level> {
        let attribute = |name: &str| {
            element
                .get_attributes()
                .iter()
                .find(|attribute| attribute.get_name() == name)
                .map(|attribute| attribute.get_value().as_str())
        };
        let hint = |name: &str| attribute(name).map_or(Dimension::Auto, parse_dimension_attribute);
        let mut control = None;
        let (key, natural) = match (element.get_name().as_str(), element.get_namespace()) {
            ("input", html::Namespace::Html) => {
                let kind = match attribute("type") {
                    Some(kind) if kind.eq_ignore_ascii_case("checkbox") => ControlKind::Checkbox,
                    Some(kind) if kind.eq_ignore_ascii_case("radio") => ControlKind::Radio,
                    _ => return None,
                };
                control = Some(ToggleControl {
                    kind,
                    checked: attribute("checked").is_some(),
                    disabled: attribute("disabled").is_some(),
                });
                (
                    None,
                    NaturalSize::from_size(crate::Size {
                        width: CHECKBOX_RADIO_SIZE,
                        height: CHECKBOX_RADIO_SIZE,
                    }),
                )
            }
            ("img", html::Namespace::Html) => {
                let src = attribute("src").map(str::trim).unwrap_or_default();
                let key = ImageKey::Src(src.to_owned());
                let size = (!src.is_empty())
                    .then(|| self.images.natural_size(&key))
                    .flatten();
                let natural = match size {
                    Some(size) => NaturalSize::from_size(size),
                    None => {
                        let alt = attribute("alt").map(str::trim).unwrap_or_default();
                        let sized = style.width != Dimension::Auto
                            || style.height != Dimension::Auto
                            || hint("width") != Dimension::Auto
                            || hint("height") != Dimension::Auto;
                        log::debug!("image {src:?} for node {node:?} is not available");
                        if !alt.is_empty() && !sized {
                            BuildStats::bump(&self.stats.text_runs);
                            return Some(Level::Inline(InlineItem::Text {
                                node,
                                element: Some(node),
                                text: style.text_transform.apply(alt),
                                style: Rc::clone(style),
                                white_space: WhiteSpace::Collapse,
                            }));
                        }
                        NaturalSize {
                            width: Some(0.0),
                            height: Some(0.0),
                            ratio: None,
                        }
                    }
                };
                (Some(key), natural)
            }
            ("svg", html::Namespace::Svg) => {
                let nested = self
                    .document
                    .node(node)
                    .and_then(|dom_node| dom_node.get_parent())
                    .and_then(|parent| self.document.node(parent))
                    .is_some_and(|parent| {
                        matches!(
                            parent.get_kind(),
                            NodeKind::Element(parent) if *parent.get_namespace() == html::Namespace::Svg
                        )
                    });
                if nested {
                    return None;
                }
                // The parser lowercases attribute names, so `viewBox` is
                // stored as `viewbox`.
                let absolute = |name: &str| match hint(name) {
                    Dimension::Px(px) => Some(px),
                    _ => None,
                };
                let ratio = attribute("viewbox").and_then(view_box_ratio);
                let natural = NaturalSize {
                    width: absolute("width"),
                    height: absolute("height"),
                    ratio: ratio.or(match (absolute("width"), absolute("height")) {
                        (Some(width), Some(height)) if height > 0.0 => Some(width / height),
                        _ => None,
                    }),
                };
                (Some(ImageKey::InlineSvg(node)), natural)
            }
            _ => return None,
        };
        BuildStats::bump(&self.stats.boxes);
        log::trace!(
            "replaced box for node {node:?} <{}>: {key:?}, natural {natural:?}",
            element.get_name()
        );
        let block = BlockBox {
            node: Some(node),
            style: Rc::clone(style),
            establishes_bfc: true,
            clips_contents: false,
            centers_lines: false,
            contents: BlockContents::Replaced(ReplacedBox {
                key,
                control,
                natural,
                // The `width` and `height` attributes do not apply to
                // checkbox and radio inputs.
                width_hint: if control.is_some() {
                    Dimension::Auto
                } else {
                    hint("width")
                },
                height_hint: if control.is_some() {
                    Dimension::Auto
                } else {
                    hint("height")
                },
            }),
            control: None,
        };
        Some(level_for(block))
    }

    /// The box for a form control whose visible text the user agent draws,
    /// or `None` for other elements.
    ///
    /// - Text-like `<input>`s (`text`, `search`, `email`, `url`, `tel`,
    ///   `password`, `number`, and a missing or unknown `type`) show their
    ///   `value` attribute with line breaks removed, or, when it is missing
    ///   or empty, their `placeholder` in [`PLACEHOLDER_COLOR`]. A `password`
    ///   input shows one `•` per character of its value instead.
    /// - `<textarea>` shows its child text, or its `placeholder` when it has
    ///   no text, as `white-space: pre-wrap` ([`WhiteSpace::PreWrap`]): line
    ///   breaks and spaces are kept and long lines wrap.
    /// - `<input type=submit|reset|button>` shows its `value` (even when
    ///   empty), or else "Submit", "Reset", or nothing, on one line. The
    ///   defaults are not localized.
    ///
    /// Single-line text drops CR and LF and shows tabs as one space. An
    /// empty single-line control is still one line tall.
    ///
    /// Other input types (`image`, `file`, checkboxes, dates, …) return
    /// `None` and keep their empty inline-block box. Values changed at runtime, the
    /// caret, and selection are not supported.
    fn form_control(
        &self,
        node: NodeID,
        element: &html::ElementData,
        style: &Rc<ComputedStyle>,
    ) -> Option<Level> {
        if *element.get_namespace() != html::Namespace::Html {
            return None;
        }
        let attribute = |name: &str| {
            element
                .get_attributes()
                .iter()
                .find(|attribute| attribute.get_name() == name)
                .map(|attribute| attribute.get_value().as_str())
        };
        // Inputs drop line breaks from the placeholder; textareas keep them.
        let placeholder = |single_line: bool| {
            let text: String = attribute("placeholder")
                .unwrap_or_default()
                .chars()
                .filter(|character| !single_line || !matches!(character, '\n' | '\r'))
                .collect();
            let style = Rc::new(ComputedStyle {
                color: PLACEHOLDER_COLOR,
                ..ComputedStyle::clone(style)
            });
            (text, style)
        };
        let block = match element.get_name().as_str() {
            "input" => {
                let kind = attribute("type").unwrap_or_default().to_ascii_lowercase();
                let button_label = button_input_default_label(&kind);
                if button_label.is_none() && !is_text_like_input_type(&kind) {
                    return None;
                }
                let value = single_line_text(attribute("value").unwrap_or_default());
                let (text, text_style) = if let Some(default_label) = button_label {
                    // The label is the value, even when empty.
                    let label =
                        attribute("value").map_or_else(|| default_label.to_owned(), |_| value);
                    (label, Rc::clone(style))
                } else if value.is_empty() {
                    placeholder(true)
                } else if kind == "password" {
                    ("\u{2022}".repeat(value.chars().count()), Rc::clone(style))
                } else {
                    (value, Rc::clone(style))
                };
                self.control_text_box(node, style, &text, &text_style)
            }
            "textarea" => {
                let mut text = String::new();
                for child in self
                    .document
                    .node(node)
                    .map(|dom_node| dom_node.get_children().as_slice())
                    .unwrap_or_default()
                {
                    if let Some(NodeKind::Text(data)) =
                        self.document.node(*child).map(|child| child.get_kind())
                    {
                        text.push_str(data);
                    }
                }
                let (text, text_style) = if text.is_empty() {
                    placeholder(false)
                } else {
                    (text, Rc::clone(style))
                };
                let text = text.replace("\r\n", "\n").replace('\r', "\n");
                let mut items = Vec::new();
                for (index, line) in text.split('\n').enumerate() {
                    if index > 0 {
                        items.push(InlineItem::ForcedBreak);
                    }
                    items.extend(control_text_item(
                        node,
                        line,
                        &text_style,
                        WhiteSpace::PreWrap,
                    ));
                }
                BuildStats::bump(&self.stats.boxes);
                BlockBox {
                    node: Some(node),
                    style: Rc::clone(style),
                    establishes_bfc: true,
                    clips_contents: true,
                    centers_lines: false,
                    control: self.form_control_sizing(node),
                    contents: BlockContents::Inlines(items),
                }
            }
            _ => return None,
        };
        log::trace!(
            "form control box for node {node:?} <{}>",
            element.get_name()
        );
        Some(level_for(block))
    }

    /// Builds the box of a single-line form control that shows `text`, text
    /// the user agent draws rather than DOM children (an input's value or
    /// placeholder, or a button-like input's label).
    ///
    /// `style` is the control's computed style and sizes the box;
    /// `text_style` styles the text run, so callers can recolor it (as
    /// placeholders do) without changing the box. The text is kept on one
    /// line with its spaces preserved (as [`WhiteSpace::Pre`]),
    /// centered vertically in the content box, and clipped to the padding
    /// box. Horizontal placement follows the control's `text-align`. Empty
    /// `text` gives a box whose only line is an empty strut, so it is still
    /// one line tall. `text-transform` is applied here.
    ///
    /// Wrap the result with [`level_for`] (or `InlineItem::Atomic`) to place
    /// it.
    pub(crate) fn control_text_box(
        &self,
        node: NodeID,
        style: &Rc<ComputedStyle>,
        text: &str,
        text_style: &Rc<ComputedStyle>,
    ) -> BlockBox {
        BuildStats::bump(&self.stats.boxes);
        BlockBox {
            node: Some(node),
            style: Rc::clone(style),
            establishes_bfc: true,
            clips_contents: true,
            centers_lines: true,
            control: self.form_control_sizing(node),
            contents: BlockContents::Inlines(
                control_text_item(node, text, text_style, WhiteSpace::Pre)
                    .inspect(|_| BuildStats::bump(&self.stats.text_runs))
                    .into_iter()
                    .collect(),
            ),
        }
    }
}

/// The color of placeholder text: Chrome's default `::placeholder` color,
/// `#757575`. The `::placeholder` pseudo-element is not supported, so pages
/// cannot restyle it.
pub(crate) const PLACEHOLDER_COLOR: css::CssColor = css::CssColor {
    r: 0x75,
    g: 0x75,
    b: 0x75,
    a: 255,
};

/// `text` for one line of a control: CR and LF are dropped, and each tab
/// shows as one space because tab stops are not supported.
fn single_line_text(text: &str) -> String {
    text.chars()
        .filter(|character| !matches!(character, '\n' | '\r'))
        .map(|character| if character == '\t' { ' ' } else { character })
        .collect()
}

/// The label an `<input>` button with the lowercased `type` value `kind`
/// shows when it has no `value` attribute: "Submit" for `submit`, "Reset"
/// for `reset`, and no text for `button`. `None` for other types. The
/// labels are not localized; `type=image` is not a labelled button here.
fn button_input_default_label(kind: &str) -> Option<&'static str> {
    match kind {
        "submit" => Some("Submit"),
        "reset" => Some("Reset"),
        "button" => Some(""),
        _ => None,
    }
}

/// Whether an `<input>` with the lowercased `type` value `kind` is a text
/// field. Missing, empty, and unknown types are `text` per HTML.
fn is_text_like_input_type(kind: &str) -> bool {
    !matches!(
        kind,
        "hidden"
            | "checkbox"
            | "radio"
            | "file"
            | "image"
            | "submit"
            | "reset"
            | "button"
            | "color"
            | "range"
            | "date"
            | "month"
            | "week"
            | "time"
            | "datetime-local"
    )
}

/// One text run for user-agent-drawn control text, or `None` when
/// `text` is empty. The run's `node` and `element` are the control, which is
/// also its hit-test target.
fn control_text_item(
    node: NodeID,
    text: &str,
    style: &Rc<ComputedStyle>,
    white_space: WhiteSpace,
) -> Option<InlineItem> {
    (!text.is_empty()).then(|| InlineItem::Text {
        node,
        element: Some(node),
        text: style.text_transform.apply(text),
        style: Rc::clone(style),
        white_space,
    })
}

/// Places a box at its outer display type: block-level boxes become
/// [`Level::Block`], everything else an atomic inline.
fn level_for(block: BlockBox) -> Level {
    match block.style.display {
        Display::Box {
            outside: DisplayOutside::Block,
            ..
        } => Level::Block(block),
        _ => Level::Inline(InlineItem::Atomic(block)),
    }
}

/// Pushes a block-level box. A block inside inline boxes splits them around
/// it (CSS 2.2 §9.2.1.1).
fn split_inlines_around(
    block: BlockBox,
    levels: &mut Vec<Level>,
    open_inlines: &[(NodeID, Rc<ComputedStyle>)],
) {
    for _ in open_inlines {
        levels.push(Level::Inline(InlineItem::End { end_edge: false }));
    }
    levels.push(Level::Block(block));
    for (open, open_style) in open_inlines {
        levels.push(Level::Inline(InlineItem::Start {
            node: *open,
            style: Rc::clone(open_style),
            start_edge: false,
        }));
    }
}

/// Whether an item is text consisting only of collapsible white space.
///
/// `white-space` is not supported yet, so all text uses `normal`, which
/// collapses spaces, tabs, and segment breaks.
fn is_collapsible_whitespace(item: &InlineItem) -> bool {
    match item {
        InlineItem::Text { text, .. } => text.chars().all(is_document_white_space),
        _ => false,
    }
}

/// Whether an item is a floated or absolutely positioned box.
fn is_out_of_flow(item: &InlineItem) -> bool {
    matches!(item, InlineItem::OutOfFlow(_))
}

/// Document white space for CSS Text 3 §4: space, tab, and segment breaks.
pub(crate) fn is_document_white_space(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\n' | '\r')
}
