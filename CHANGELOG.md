# Changelog

Notable changes to the Nib workspace. Versions follow semantic versioning;
releases before 1.2.0 are described by their git tags.

## [Unreleased]

## [1.2.1] - 2026-10-06

### Changed

- The minimum supported Rust version is 1.99.0, raised from 1.98.1. The pinned toolchain and `rust-version` move together, so the workspace no longer builds on an older compiler.
- `nib-html` parses with html5ever 0.40.1 and keeps the parsed tree in a structure of its own. `markup5ever_rcdom`, which had no release for html5ever 0.40, is no longer a dependency, and `xml5ever` goes with it. The public API is unchanged, and so is every document: the new and old parsers agree on all 1,792 inputs of the HTML tree-construction conformance tests and on 250,000 generated ones.
- HTML nested tens of thousands of block elements deep parses in about half the time: 50,000 nested `<div>`s took 4.2 s and take 2.1 s. The time that remains is the HTML parser's own, which looks back up its stack of open elements for every block-level tag.

### Fixed

- HTML: a `<meta http-equiv="Content-Type">` whose `content` stopped at the word `charset` — `content="text/html; charset"` — made the HTML parser read past the end of the value and panic, which in a mail reader is a crash on opening the message. It parses.

## [1.2.0] - 2026-09-29

### Added

- `nib_model::link::is_followable` and `FOLLOWABLE_SCHEMES`: the one link-target policy the HTML parser, the Markdown parser and the widget share.
- `nib_html::parse_with_report` (and `Html::parse_with_report`, `styling::read`) returns a `Report` with every attempt to hide text and every refused declaration — the accounting that was computed and then thrown away.
- The widget's clipboard carries HTML: a copy offers `text/html` beside plain text, so structure survives a paste into another application, and a paste from one reads its HTML through `nib-html`'s allow-list and link policy. The `clipboard` feature no longer pulls in the unused `nib-text`.

### Changed

- Markdown export writes a line break as `\` at the end of the line rather than two trailing spaces, which a line of nothing but spaces turned into a paragraph break.

### Fixed

- HTML nested deeper than 256 elements no longer overflows the stack and aborts the process. The rest of such a subtree is read as text, and ignored elements inside it (`<script>`, `<style>`) stay dropped.
- Links read from HTML keep only `http`, `https`, `mailto`, `tel`, `ftp` and relative targets. `javascript:`, `data:`, `file:`, network-share and any other scheme lose the link and keep the text.
- Paragraph and heading alignment is written back out to HTML as `text-align`, so an HTML round trip keeps it.
- CSS values carrying `!important` are read: `color: red !important` gives a colour, and `display: none !important` is reported as hiding.
- A unitless `font-size: 0` is reported as hiding text.
- MDX and MDC: lines inside a fenced code block are no longer taken as components or module lines, which deleted the code.
- Markdown export of code that contains backticks no longer ends the code block or span early.
- Markdown export keeps link and image titles, and destinations containing spaces or parentheses.
- Markdown export escapes literal text that would read back as HTML, an entity, an ordered list, strikethrough, a setext heading or a table cell break.
- A blockquote of several paragraphs stays one quote through Markdown export.
- A list inside a quote keeps its markers in order (`> - one`) in Markdown and plain-text output.
- Plain-text parsing keeps the `-- ` signature separator, so a draft reopened and written out again still carries it.
- Undo after a change made outside the history, before the history's own changes, takes back the right text and restores the right selection.
- Spellcheck: `word_at` gives the right range after a line break or other inline atom, and a word split by formatting is checked as one word.
- Links read from Markdown follow the same scheme policy as HTML: `javascript:`, `file:`, `data:` and other unlisted schemes lose the link and keep the text.
- The widget reports link clicks as `Action::Link`, which it never emitted, so links in readers did nothing. A click follows a link in a read-only editor and Ctrl+click does in an editable one; a press that becomes a drag selects instead. The pointer turns to a hand over a link a click would follow.
- Markdown nested deeper than 256 levels — fifty thousand `>` on a line — no longer overflows the stack and aborts the process. Deeper containers are read as transparent and their text is kept.
- Markdown: bold, italics, code spans and links in a tight list item (the usual kind) were dropped when the file was read; they are kept.
- HTML: inline markup outside a paragraph — `<b>x</b>` on its own, `<li><em>x</em></li>`, a browser's clipboard fragment — kept its text but lost its marks; it keeps both.
- HTML: blocks the parser nested inside an unclosed `<pre>` could give a document the schema forbids (a list item inside a list item, an image directly in a quote), and ordinary paragraphs there kept their raw whitespace. Placement is checked again after every close, and only code keeps whitespace as written. Two runs of text no longer meet with a doubled space.
- Plain text: an empty list item inside a quote was written `> - >`; it is written as its marker alone, with no trailing space for a flowed-text reader to take as a soft break.
- Markdown: saving a document and opening it again gives back the same document for everything the new generated round-trip tests produce. Code keeps its trailing spaces and gains no newline; lists side by side stay separate; line breaks in a row stay in their paragraph; `!` before a link, `#` ending a heading and entities in link targets, titles and code languages are escaped; empty items, task items with more than one paragraph and nested lists that cannot interrupt a paragraph keep their shape. Emphasis that no `*` or `~~` can express is written as `<strong>`, `<em>` or `<del>`, and those tags — with `<b>`, `<i>`, `<s>` — are read as emphasis, ending with their block.
- Copying a selection inside one paragraph — the ordinary copy of a word or phrase — put an empty string on the system clipboard; `nib::plain_text` returns its text.
