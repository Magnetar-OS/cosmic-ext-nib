# Changelog

Notable changes to the Nib workspace. Versions follow semantic versioning;
releases before 1.1.1 are described by their git tags.

## [Unreleased]

### Added

- `nib_model::link::is_followable` and `FOLLOWABLE_SCHEMES`: the one link-target policy the HTML parser, the Markdown parser and the widget share.
- `nib_html::parse_with_report` (and `Html::parse_with_report`, `styling::read`) returns a `Report` with every attempt to hide text and every refused declaration — the accounting that was computed and then thrown away.

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
