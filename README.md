# Nib

A rich text engine for COSMIC — the document model, the schema, the changes,
and the widget that edits them, in Rust, on iced, with no web engine anywhere
in the pipeline.

TipTap is the benchmark: a document model that cannot represent an invalid
document, an extension API that adds node types and commands without forking
the core, and a view that is a real editor rather than a text box with a
toolbar over it. What TipTap gets for free from the browser — layout, hit
testing, `contenteditable` — has to be built here. Everything else is a data
structure, and data structures port.

## The seam

```
  ┌─ nib ──────────────────── the libcosmic/iced widget: layout, caret,
  │                           selection, input, clipboard, decorations
  ├─ nib-html ─────────────── HTML in and out (html5ever)
  ├─ nib-markdown ─────────── Markdown in and out
  ├─ nib-text ─────────────── plain text out, mail quoting in
  └─ nib-model ───────────── the document: schema, positions, steps,
                              transactions. No toolkit. No pixels.
```

`nib-model` is the load-bearing crate and it knows nothing about display. That
is not modesty about scope, it is the property that makes the rest possible: a
model with no renderer is exhaustively testable in milliseconds, runs on a
server with no display attached, and survives the view layer being replaced.

The design is ProseMirror's, because ProseMirror got the hard parts right and
there is no Rust prior art to fork. This is a port of its ideas, not its code.

## The four ideas

**A schema, enforced.** A schema declares the node and mark types a document
may contain and what each may contain in turn. Each content expression —
`block+`, `inline*`, `paragraph block*` — compiles to a DFA, so the engine can
answer not just *"is this allowed"* but *"what would make this allowed"*. That
second question is the one that matters: pressing Enter at the end of a list
item, pasting a heading into a table cell and deleting the last row of a table
are all answered by searching that automaton, and none of them can be answered
by a boolean. The cost is a real regular-expression compiler in `content.rs`;
the return is that a list item outside a list is not a bug to fix but a state
the model cannot reach.

**Immutable, structurally shared nodes.** Every edit returns a new document
that shares every subtree it did not touch. A keystroke rebuilds one paragraph
and the spine of ancestors above it — a handful of small allocations — and
leaves the other thousand paragraphs pointed at by both versions. That is what
makes an undo entry a pointer rather than a copy, and what lets the view answer
"did this subtree change?" with one `Arc::ptr_eq`.

**Flat integer positions.** A position is a count from the start of the
document, not a path. Paths are what the tree *is*, but every edit invalidates
every path after it, and the ones nobody remembered to fix are the bugs where
the cursor jumps a paragraph when a collaborator types. An integer has one rule
— map it through the change — implemented once, and applied identically to the
caret, a selection, a comment anchor, a decoration and a remote user's cursor.

**Steps that invert and map.** A change is a value. It applies to a document,
it inverts against the document it applied to, and it maps through other
changes. Undo is the second property. Collaborative editing is the third. They
are not two mechanisms; they are two uses of the same two functions.

## Decisions worth stating

**Positions are UTF-8 byte offsets.** ProseMirror counts UTF-16 code units
because it lives in a browser. Counting bytes means a text node's size is
`str::len()`, a cut is a `&str` slice, and the offsets handed to the shaper
need no conversion — `cosmic-text` also works in bytes. The cost is that a
position can name a place inside a character, so every cut asserts on a
character boundary rather than rounding to one. A silent round turns a mapping
bug into mojibake three edits later; a panic puts it where it happened.

**The step set is closed.** ProseMirror lets applications define step types.
This does not, because every peer in a collaborative session and every reader
of a stored history must be able to apply and invert every step it receives,
and a refusal in the middle of a rebase is not recoverable. The set is complete
without extension: `ReplaceAround` expresses any structural change — wrap,
lift, split, join, retype a block — as one atomic invertible operation.

**Node types are compared by identity.** Documents built against two separately
built `Schema`s are not interchangeable, even when the two were declared
identically. Hold one `Schema` and clone it; it is `Arc`-backed and cloning is
a pointer bump. `basic::schema()` hands out a single shared instance for
exactly this reason.

**The editor is not a renderer of arbitrary HTML.** TipTap does not edit
arbitrary HTML either — it edits a schema-constrained document and *emits*
HTML. Mail HTML that does not fit the schema is a reading problem, not an
editing one, and belongs to whatever renders it; the composer quotes it as an
opaque island. The two meet at the HTML serialiser, not in the layout engine.

## State

`nib-model` is written and tested: schema and content-expression compiler, the
node tree, marks, position resolution, slices, the replace algorithm, position
mapping, the step set, and the structural edits (`split`, `join`, `lift`,
`wrap`, `set_block_type`, `set_node_markup`, `clear_incompatible`, `add_mark`,
`remove_mark`). 112 tests, no warnings under `clippy --all-targets`.

Still to come, in order: the slice fitter that paste needs; editor state,
selections and plugins; history, commands, keymaps and input rules; the
extension API; the HTML, Markdown and text serialisers; and the widget.

## Building

```
just test     # cargo test --workspace --locked
just check    # clippy, pedantic, as warnings
```

## Licence

MPL-2.0, like libcosmic whose patterns this follows and like the `cosmic-pim`
substrate. Applications that link Nib stay whatever licence they are; the
file-level copyleft asks only that changes to *these* files come back.
