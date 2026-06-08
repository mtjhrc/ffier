# Format, lint, and test — run before committing
check: fmt clippy test

# Format all code
fmt:
    cargo fmt

# Run clippy
clippy:
    cargo clippy --workspace -- -D warnings

# Run all tests
test:
    just --justfile tests/justfile
