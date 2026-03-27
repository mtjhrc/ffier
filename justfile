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
    cc -Wall -Wextra -Werror -g -o tests/c/test_main tests/c/test_main.c \
        -I tests/c \
        -L target/debug \
        -lffier_test_cdylib \
        -Wl,-rpath,$(pwd)/target/debug
    ./tests/c/test_main

# Run C tests under valgrind (memcheck + leak check + uninitialized value tracking)
valgrind: test-c
    valgrind --leak-check=full --show-leak-kinds=all --track-origins=yes --error-exitcode=1 \
        ./tests/c/test_main

# Run Miri with Stacked Borrows (default model)
miri-stacked:
    cargo +nightly miri test -p ffier-test-cdylib

# Run Miri with Tree Borrows
miri-tree:
    MIRIFLAGS="-Zmiri-tree-borrows" cargo +nightly miri test -p ffier-test-cdylib

# Run Miri with both memory models
miri: miri-stacked miri-tree

# Run everything
test: check-header valgrind miri
    @echo ""
    @echo "All checks passed!"
