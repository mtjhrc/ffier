# Format, lint, and test — run before committing
check: fmt-check clippy test

# Check formatting (fails if files need formatting)
fmt-check:
    cargo fmt -- --check

# Format all code
fmt:
    cargo fmt

# Run clippy
clippy:
    cargo clippy --workspace -- -D warnings

# Run all tests
test:
    just --justfile tests/justfile
