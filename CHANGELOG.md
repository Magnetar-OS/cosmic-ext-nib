# Changelog

Notable changes to the Nib workspace. Versions follow semantic versioning;
releases before 1.1.1 are described by their git tags.

## [Unreleased]

### Added

- `nib_model::link::is_followable` and `FOLLOWABLE_SCHEMES`: the one link-target policy the HTML parser, the Markdown parser and the widget share.

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
