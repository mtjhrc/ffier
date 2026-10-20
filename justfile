# Format, lint, and test — run before committing
check: fmt-check clippy test

# Check formatting (fails if files need formatting)
fmt-check:
    cargo fmt -- --check
    cargo fmt --manifest-path tests/conditional-bridge/Cargo.toml --all -- --check

# Format all code
fmt:
    cargo fmt
    cargo fmt --manifest-path tests/conditional-bridge/Cargo.toml --all

# Run clippy
# Builds the test cdylibs first: the client crates' build scripts emit
# link directives for them, so a fresh checkout cannot type-check the
# workspace until the .so files exist.
clippy:
    just --justfile tests/justfile build
    cargo clippy --workspace -- -D warnings
    cargo clippy --manifest-path tests/conditional-bridge/Cargo.toml \
        --target-dir target/conditional-bridge --workspace --all-targets \
        --features conditional-bridge/a_feature -- -D warnings
    cargo clippy --manifest-path tests/conditional-bridge/Cargo.toml \
        --target-dir target/conditional-bridge --workspace --all-targets -- -D warnings

# Run all tests
test:
    just --justfile tests/justfile
