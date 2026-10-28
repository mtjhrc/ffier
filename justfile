set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]
python := if os() == "windows" { "python" } else { "python3" }

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
# workspace until the shared libraries exist.
clippy:
    {{python}} tests/run.py clippy

# Run all tests
test:
    {{python}} tests/run.py all
