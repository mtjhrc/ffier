CARGO_TARGET_DIR := ../../target
LIB_DIR := $(CARGO_TARGET_DIR)/debug

.PHONY: all clean run

all: main

# Build the cdylib and generate the header
$(LIB_DIR)/libexample_cdylib.so: ../cdylib/src/lib.rs ../lib/src/lib.rs
	cargo build -p example-cdylib --manifest-path ../../Cargo.toml

mylib.h: ../cdylib/src/gen_header.rs ../lib/src/lib.rs
	cargo run -p example-cdylib --bin gen-header --manifest-path ../../Cargo.toml > $@

main: main.c mylib.h $(LIB_DIR)/libexample_cdylib.so
	$(CC) -o $@ main.c -L$(LIB_DIR) -Wl,--no-as-needed -lexample_cdylib

run: main
	LD_LIBRARY_PATH=$(LIB_DIR) ./main

clean:
	rm -f main mylib.h
