# Generate the C header from the Rust cdylib
gen-header:
    cargo run --manifest-path tests/cdylib/Cargo.toml --bin gen-header > tests/c/ffier_test.h

# Update the expected header snapshot
update-expected-header: gen-header
    cp tests/c/ffier_test.h tests/expected_header.h

# Check header matches expected (byte-for-byte)
check-header: gen-header
    diff tests/expected_header.h tests/c/ffier_test.h

# Build the cdylib
build-cdylib:
    cargo build --manifest-path tests/cdylib/Cargo.toml

# Compile and run the C tests
test-c: build-cdylib gen-header
    cc -Wall -Wextra -Werror -o tests/c/test_main tests/c/test_main.c \
        -I tests/c \
        -L target/debug \
        -lffier_test_cdylib \
        -Wl,-rpath,$(pwd)/target/debug
    ./tests/c/test_main

# Run everything
test: check-header test-c
    @echo ""
    @echo "All checks passed!"
