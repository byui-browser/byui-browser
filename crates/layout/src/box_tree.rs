//! Box generation: from the DOM and computed styles to the box tree
//! (CSS Display 3 §2, CSS 2.2 §9.2).

use std::rc::Rc;

use html::{HTMLDocument, NodeID, NodeKind};

use crate::style::{ComputedStyle, Display, DisplayInside, DisplayOutside};

/// A block container box before layout.
#[derive(Debug)]
pub(crate) struct BlockBox {
    /// The generating element, or `None` for anonymous boxes and the initial
    /// containing block.
    pub node: Option<NodeID>,
    pub style: Rc<ComputedStyle>,
    /// Whether the box establishes a new block formatting context
    /// (CSS 2.2 §9.4.1), which stops margins collapsing through it.
    pub establishes_bfc: bool,
    pub contents: BlockContents,
}

/// A block container holds either only block-level boxes or only inline-level
/// content (CSS 2.2 §9.2.1.1).
#[derive(Debug)]
pub(crate) enum BlockContents {
    Blocks(Vec<BlockBox>),
    Inlines(Vec<InlineItem>),
}

/// Inline-level content of an inline formatting context, flattened in tree
/// order. Inline boxes are bracketed by `Start` and `End`.
#[derive(Debug)]
pub(crate) enum InlineItem {
    Text {
        node: NodeID,
        element: Option<NodeID>,
        text: String,
        style: Rc<ComputedStyle>,
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
}

/// Either kind of child produced while walking a block container's children.
enum Level {
    Block(BlockBox),
    Inline(InlineItem),
}

/// Computed styles for every element, indexed by DOM arena index.
pub(crate) struct StyleMap {
    styles: Vec<Option<Rc<ComputedStyle>>>,
    initial: Rc<ComputedStyle>,
}

impl StyleMap {
    /// Resolves every element's computed style. The root element's display is
    /// blockified (CSS Display 3 §2.8).
    pub(crate) fn new(
        document: &HTMLDocument,
        styles: &css::ComputedStyles,
        root: Option<NodeID>,
        viewport: crate::Size,
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
        let resolved = (0..document.nodes.len())
            .map(|index| {
                let node = NodeID::new(index);
                let declarations = styles.declarations_for(node)?;
                let mut style =
                    ComputedStyle::from_declarations(declarations, root_font_size, viewport);
                if Some(node) == root {
                    style.display = style.display.blockified();
                }
                Some(Rc::new(style))
            })
            .collect();
        Self {
            styles: resolved,
            initial: Rc::new(ComputedStyle::anonymous(None)),
        }
    }

    pub(crate) fn get(&self, node: NodeID) -> Option<&Rc<ComputedStyle>> {
        self.styles.get(node.get())?.as_ref()
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
        .children
        .iter()
        .copied()
        .find(|child| {
            matches!(
                document.node(*child).map(|node| &node.kind),
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
) -> BlockBox {
    let builder = Builder { document, styles };
    let style = Rc::clone(styles.initial());
    let children = document
        .node(document.root)
        .map(|node| node.children.as_slice())
        .unwrap_or_default();
    BlockBox {
        node: None,
        contents: builder.container_contents(children, &style),
        style,
        establishes_bfc: true,
    }
}

struct Builder<'a> {
    document: &'a HTMLDocument,
    styles: &'a StyleMap,
}

impl Builder<'_> {
    /// Builds the box for an element whose outer display is block or whose
    /// inner display makes it atomic.
    fn block_box(&self, node: NodeID, style: &Rc<ComputedStyle>, is_root: bool) -> BlockBox {
        let flow_root = matches!(
            style.display,
            Display::Box {
                inside: DisplayInside::FlowRoot,
                ..
            }
        );
        let children = self
            .document
            .node(node)
            .map(|node| node.children.as_slice())
            .unwrap_or_default();
        BlockBox {
            node: Some(node),
            contents: self.container_contents(children, style),
            style: Rc::clone(style),
            establishes_bfc: is_root || flow_root,
        }
    }

    /// Boxes a block container's children, wrapping inline-level runs in
    /// anonymous block boxes when block-level siblings exist.
    fn container_contents(&self, children: &[NodeID], style: &Rc<ComputedStyle>) -> BlockContents {
        let mut levels = Vec::new();
        let mut open_inlines = Vec::new();
        for child in children {
            self.collect(*child, &mut levels, &mut open_inlines);
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
            if items.iter().all(is_collapsible_whitespace) {
                // White space that would be collapsed away generates no
                // anonymous inline boxes (CSS 2.2 §9.2.2.1).
                return;
            }
            blocks.push(BlockBox {
                node: None,
                style: Rc::new(ComputedStyle::anonymous(Some(style))),
                establishes_bfc: false,
                contents: BlockContents::Inlines(items),
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

    fn collect(
        &self,
        node: NodeID,
        levels: &mut Vec<Level>,
        open_inlines: &mut Vec<(NodeID, Rc<ComputedStyle>)>,
    ) {
        let Some(dom_node) = self.document.node(node) else {
            return;
        };
        match &dom_node.kind {
            NodeKind::Text(text) => levels.push(Level::Inline(InlineItem::Text {
                node,
                element: dom_node
                    .parent
                    .filter(|parent| self.styles.get(*parent).is_some()),
                text: text.clone(),
                style: self.styles.for_text_parent(dom_node.parent),
            })),
            NodeKind::Element(element) => {
                let Some(style) = self.styles.get(node) else {
                    return;
                };
                match style.display {
                    Display::None => {}
                    Display::Contents => {
                        for child in &dom_node.children {
                            self.collect(*child, levels, open_inlines);
                        }
                    }
                    Display::Box {
                        outside: DisplayOutside::Inline,
                        inside: DisplayInside::Flow,
                    } => {
                        if element.name == "br" {
                            levels.push(Level::Inline(InlineItem::ForcedBreak));
                            return;
                        }
                        levels.push(Level::Inline(InlineItem::Start {
                            node,
                            style: Rc::clone(style),
                            start_edge: true,
                        }));
                        open_inlines.push((node, Rc::clone(style)));
                        for child in &dom_node.children {
                            self.collect(*child, levels, open_inlines);
                        }
                        open_inlines.pop();
                        levels.push(Level::Inline(InlineItem::End { end_edge: true }));
                    }
                    Display::Box {
                        outside: DisplayOutside::Inline,
                        inside: DisplayInside::FlowRoot,
                    } => {
                        let atomic = self.block_box(node, style, false);
                        levels.push(Level::Inline(InlineItem::Atomic(atomic)));
                    }
                    Display::Box {
                        outside: DisplayOutside::Block,
                        ..
                    } => {
                        let is_root = dom_node.parent == Some(self.document.root)
                            && root_element(self.document) == Some(node);
                        // A block inside inline boxes splits them around it
                        // (CSS 2.2 §9.2.1.1).
                        for _ in open_inlines.iter() {
                            levels.push(Level::Inline(InlineItem::End { end_edge: false }));
                        }
                        levels.push(Level::Block(self.block_box(node, style, is_root)));
                        for (open, open_style) in open_inlines.iter() {
                            levels.push(Level::Inline(InlineItem::Start {
                                node: *open,
                                style: Rc::clone(open_style),
                                start_edge: false,
                            }));
                        }
                    }
                }
            }
            _ => {}
        }
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

/// Document white space for CSS Text 3 §4: space, tab, and segment breaks.
pub(crate) fn is_document_white_space(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\n' | '\r')
}
