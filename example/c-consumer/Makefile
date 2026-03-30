CARGO_TARGET_DIR := ../../target
LIB_DIR := $(CARGO_TARGET_DIR)/debug

.PHONY: all clean

all: main

# Build the cdylib and generate the header
$(LIB_DIR)/libmylib_cdylib.so: ../mylib-cdylib/src/lib.rs ../mylib/src/lib.rs
	cargo build -p mylib-cdylib --manifest-path ../../Cargo.toml

mylib.h: ../mylib-cdylib/src/gen_header.rs ../mylib/src/lib.rs
	cargo run -p mylib-cdylib --bin gen-header --manifest-path ../../Cargo.toml > $@

main: main.c mylib.h $(LIB_DIR)/libmylib_cdylib.so
	$(CC) -o $@ main.c -L$(LIB_DIR) -Wl,--no-as-needed -lmylib_cdylib

clean:
	rm -f main mylib.h
