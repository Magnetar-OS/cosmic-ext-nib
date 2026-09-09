# The conventional COSMIC build entry point, trimmed to what a library
# workspace has: no binary, no desktop entry, so no install/uninstall.

export NAME := 'cosmic-ext-nib'

default: build-release

build-debug *args:
    cargo build --workspace --locked {{args}}

build-release *args: (build-debug '--release' args)

# Pedantic is declared in Cargo.toml, so it applies here and in the editor
# alike; this only adds the targets the plain build leaves out.
check *args:
    cargo clippy --workspace --all-targets --locked {{args}}

test *args:
    cargo test --workspace --locked {{args}}

fmt:
    cargo +nightly fmt --all

# What CI runs, in the order CI runs it.
ci: && check test
    cargo fmt --all --check
