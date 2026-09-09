# The conventional COSMIC build entry point, trimmed to what a library
# workspace has: no binary, no desktop entry, so no install/uninstall.

export NAME := 'cosmic-ext-nib'

default: build-release

build-debug *args:
    cargo build --workspace --locked {{args}}

build-release *args: (build-debug '--release' args)

# Pedantic as warnings, not denials — the ecosystem standard.
check *args:
    cargo clippy --workspace --all-targets --locked {{args}} -- -W clippy::pedantic

test *args:
    cargo test --workspace --locked {{args}}

fmt:
    cargo +nightly fmt --all
