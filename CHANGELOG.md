# Changelog

Notable changes to the Nib workspace. Versions follow semantic versioning;
releases before 1.1.1 are described by their git tags.

## [Unreleased]

### Fixed

- HTML nested deeper than 256 elements no longer overflows the stack and aborts the process. The rest of such a subtree is read as text, and ignored elements inside it (`<script>`, `<style>`) stay dropped.
- Links read from HTML keep only `http`, `https`, `mailto`, `tel`, `ftp` and relative targets. `javascript:`, `data:`, `file:`, network-share and any other scheme lose the link and keep the text.
- Paragraph and heading alignment is written back out to HTML as `text-align`, so an HTML round trip keeps it.
- CSS values carrying `!important` are read: `color: red !important` gives a colour, and `display: none !important` is reported as hiding.
- A unitless `font-size: 0` is reported as hiding text.
