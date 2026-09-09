// SPDX-License-Identifier: MPL-2.0

//! Flattening a document into the boxes that get drawn.
//!
//! # Why flat, and not a tree of widgets
//!
//! The obvious shape for a block editor is a widget per block, nested the way
//! the document is. It is also the wrong shape, and the reason is the
//! selection: a selection runs from a position in one block to a position in
//! another, and the two blocks may be at different depths in different
//! ancestors. A tree of widgets makes that a conversation between widgets that
//! cannot see each other; one widget with a flat list of boxes makes it
//! arithmetic.
//!
//! So the document is walked once into a list of [`Block`]s in document order,
//! each knowing the range of positions it covers and how far in it sits. Every
//! subsequent question — where is the caret, what is selected, which block did
//! the pointer land in — is answered against that list.
//!
//! # Positions and text offsets
//!
//! A block's text is the concatenation of its inline content, and the two
//! coordinate systems do not quite line up: a text node contributes its bytes
//! to both, but an inline atom — an image, a mention — is one position in the
//! document and something else on screen. [`Segment`] records the
//! correspondence so a hit test can be turned back into a document position
//! without guessing.

use nib_model::node::Node;
use nib_model::schema::NodeTypeId;

/// What stands in for an inline atom in a block's text.
///
/// U+FFFC OBJECT REPLACEMENT CHARACTER: it has a glyph in most fonts, it takes
/// part in shaping and hit-testing like any other character, and no author
/// types it.
pub const ATOM: &str = "\u{fffc}";

/// A run of a block's text and the document positions it corresponds to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub text_from: usize,
    pub text_to: usize,
    pub doc_from: usize,
    pub doc_to: usize,
}

/// What a block is, for drawing purposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// A textblock: a paragraph, a heading, a code line, a table cell's
    /// content.
    Text,
    /// A leaf that draws itself: a rule.
    Rule,
    /// A block-level image or other atom, named so the view can decide.
    Atom { name: String },
}

/// One box in the flattened document.
#[derive(Debug, Clone)]
pub struct Block {
    /// The position immediately before the block's node.
    pub node_pos: usize,
    /// The first position inside the block.
    pub from: usize,
    /// The last position inside the block.
    pub to: usize,
    /// The node's type, so the view can style by it.
    pub typ: NodeTypeId,
    pub type_name: String,
    pub kind: Kind,
    /// How many list levels in.
    pub indent: usize,
    /// How many blockquotes deep.
    pub quote_depth: usize,
    /// A list marker, if this is the first block of a list item.
    pub marker: Option<Marker>,
    /// The block's inline content, as it will be laid out.
    pub text: String,
    /// The correspondence between `text` and document positions.
    pub segments: Vec<Segment>,
    /// The inline nodes, in order, for building styled spans.
    pub inline: Vec<Node>,
    /// The heading level, when this is one.
    pub level: Option<i64>,
    /// The code block's language, when this is one.
    pub language: Option<String>,
    /// Whether this block is inside a table cell.
    pub in_table: bool,
}

/// What precedes a list item's first block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Marker {
    Bullet,
    Number(i64),
    /// A task list checkbox, and whether it is ticked.
    Check(bool),
}

/// Walks a document into blocks, in document order.
#[must_use]
pub fn flatten(doc: &Node) -> Vec<Block> {
    let mut out = Vec::new();
    let mut walker = Walker {
        indent: 0,
        quote_depth: 0,
        in_table: false,
        marker: None,
    };
    walker.children(doc, 0, &mut out);
    out
}

struct Walker {
    indent: usize,
    quote_depth: usize,
    in_table: bool,
    /// Set by a list item for the first block inside it, then taken.
    marker: Option<Marker>,
}

impl Walker {
    /// Walks a node's children, whose content starts at `start`.
    fn children(&mut self, parent: &Node, start: usize, out: &mut Vec<Block>) {
        use nib_model::basic::nodes;

        let mut pos = start;
        for (index, child) in parent.content().into_iter().enumerate() {
            let node_pos = pos;
            let inner = pos + 1;
            match child.type_name() {
                nodes::BLOCKQUOTE => {
                    self.quote_depth += 1;
                    self.children(child, inner, out);
                    self.quote_depth -= 1;
                }
                nodes::BULLET_LIST | nodes::ORDERED_LIST => {
                    let ordered = child.type_name() == nodes::ORDERED_LIST;
                    let start_at = child.attrs().get_int("start").unwrap_or(1);
                    self.indent += 1;
                    let mut item_pos = inner;
                    for (i, item) in child.content().into_iter().enumerate() {
                        self.marker = Some(match (ordered, item.attrs().get_bool("checked")) {
                            (_, Some(checked)) => Marker::Check(checked),
                            (true, None) => {
                                Marker::Number(start_at + i64::try_from(i).unwrap_or(0))
                            }
                            (false, None) => Marker::Bullet,
                        });
                        self.children(item, item_pos + 1, out);
                        // An item whose first block never claimed the marker
                        // (an empty item) must not pass it to the next one.
                        self.marker = None;
                        item_pos += item.node_size();
                    }
                    self.indent -= 1;
                }
                nodes::TABLE => {
                    let was = self.in_table;
                    self.in_table = true;
                    let mut row_pos = inner;
                    for row in child.content() {
                        let mut cell_pos = row_pos + 1;
                        for cell in row.content() {
                            self.children(cell, cell_pos + 1, out);
                            cell_pos += cell.node_size();
                        }
                        row_pos += row.node_size();
                    }
                    self.in_table = was;
                }
                _ if child.is_textblock() => out.push(self.textblock(child, node_pos)),
                _ if child.is_leaf() => out.push(self.leaf(child, node_pos)),
                // Anything else with content: walk into it and keep going.
                _ if child.content_size() > 0 => self.children(child, inner, out),
                _ => out.push(self.leaf(child, node_pos)),
            }
            pos += child.node_size();
            let _ = index;
        }
    }

    fn textblock(&mut self, node: &Node, node_pos: usize) -> Block {
        let from = node_pos + 1;
        let mut text = String::new();
        let mut segments = Vec::new();
        let mut inline = Vec::new();
        let mut doc = from;

        for child in node.content() {
            let text_from = text.len();
            let doc_from = doc;
            if let Some(content) = child.text() {
                text.push_str(content);
            } else if child.type_name() == nib_model::basic::nodes::HARD_BREAK {
                // A newline is a break in both coordinate systems: one byte,
                // one position, and the shaper already knows what to do with
                // it.
                text.push('\n');
            } else {
                text.push_str(ATOM);
            }
            doc += child.node_size();
            segments.push(Segment {
                text_from,
                text_to: text.len(),
                doc_from,
                doc_to: doc,
            });
            inline.push(child.clone());
        }

        Block {
            node_pos,
            from,
            to: from + node.content_size(),
            typ: node.type_id(),
            type_name: node.type_name().to_owned(),
            kind: Kind::Text,
            indent: self.indent,
            quote_depth: self.quote_depth,
            marker: self.marker.take(),
            text,
            segments,
            inline,
            level: node.attrs().get_int("level"),
            language: node
                .attrs()
                .get_str("language")
                .filter(|l| !l.is_empty())
                .map(str::to_owned),
            in_table: self.in_table,
        }
    }

    fn leaf(&mut self, node: &Node, node_pos: usize) -> Block {
        let kind = if node.type_name() == nib_model::basic::nodes::HORIZONTAL_RULE {
            Kind::Rule
        } else {
            Kind::Atom {
                name: node.type_name().to_owned(),
            }
        };
        Block {
            node_pos,
            from: node_pos,
            to: node_pos + node.node_size(),
            typ: node.type_id(),
            type_name: node.type_name().to_owned(),
            kind,
            indent: self.indent,
            quote_depth: self.quote_depth,
            marker: self.marker.take(),
            text: String::new(),
            segments: Vec::new(),
            inline: Vec::new(),
            level: None,
            language: None,
            in_table: self.in_table,
        }
    }
}

impl Block {
    /// The document position a text offset corresponds to.
    #[must_use]
    pub fn doc_position(&self, offset: usize) -> usize {
        for segment in &self.segments {
            if offset < segment.text_to {
                let within = offset.saturating_sub(segment.text_from);
                // An atom is one position wide however many bytes it draws as:
                // a hit inside it lands on whichever side is nearer.
                if segment.doc_to - segment.doc_from != segment.text_to - segment.text_from {
                    let half = (segment.text_to - segment.text_from) / 2;
                    return if within < half {
                        segment.doc_from
                    } else {
                        segment.doc_to
                    };
                }
                return segment.doc_from + within;
            }
        }
        self.to
    }

    /// The text offset a document position corresponds to.
    #[must_use]
    pub fn text_offset(&self, pos: usize) -> usize {
        for segment in &self.segments {
            if pos < segment.doc_to {
                let within = pos.saturating_sub(segment.doc_from);
                if segment.doc_to - segment.doc_from != segment.text_to - segment.text_from {
                    return segment.text_from;
                }
                return segment.text_from + within;
            }
        }
        self.text.len()
    }

    /// How many `\n`-separated lines the block's text has.
    ///
    /// These are the *buffer* lines the shaper knows about — the ones a hard
    /// break makes — not the visual lines wrapping produces. Every geometry
    /// call into a paragraph is indexed by one of these.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.text.bytes().filter(|b| *b == b'\n').count() + 1
    }

    /// The byte offset a line starts at.
    #[must_use]
    pub fn line_start(&self, line: usize) -> usize {
        if line == 0 {
            return 0;
        }
        let mut seen = 0;
        for (i, byte) in self.text.bytes().enumerate() {
            if byte == b'\n' {
                seen += 1;
                if seen == line {
                    return i + 1;
                }
            }
        }
        self.text.len()
    }

    /// The byte offset a line ends at, not counting its newline.
    #[must_use]
    pub fn line_end(&self, line: usize) -> usize {
        let start = self.line_start(line);
        self.text[start..]
            .find('\n')
            .map_or(self.text.len(), |i| start + i)
    }

    /// How long a line is, in bytes.
    #[must_use]
    pub fn line_length(&self, line: usize) -> usize {
        self.line_end(line) - self.line_start(line)
    }

    /// Which line a byte offset falls on, and how far into it.
    #[must_use]
    pub fn line_of(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.text.len());
        let line = self.text[..offset].bytes().filter(|b| *b == b'\n').count();
        (line, offset - self.line_start(line))
    }

    /// True when `pos` falls inside this block's content.
    #[must_use]
    pub fn contains(&self, pos: usize) -> bool {
        pos >= self.from && pos <= self.to
    }
}
