# Changelog

Notable changes to the Nib workspace. Versions follow semantic versioning;
releases before 1.1.1 are described by their git tags.

## [Unreleased]

### Fixed

- HTML nested deeper than 256 elements no longer overflows the stack and aborts the process. The rest of such a subtree is read as text, and ignored elements inside it (`<script>`, `<style>`) stay dropped.
