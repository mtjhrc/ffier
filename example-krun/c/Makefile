CARGO_TARGET_DIR := ../../target
LIB_DIR := $(CARGO_TARGET_DIR)/debug

.PHONY: all clean run

all: main

$(LIB_DIR)/libkrun_cdylib.so: ../lib/src/lib.rs
	cargo build -p krun-cdylib --manifest-path ../../Cargo.toml

krun.h: ../cdylib/src/gen_header.rs ../lib/src/lib.rs
	cargo run -p krun-cdylib --bin krun-gen-header --manifest-path ../../Cargo.toml > $@

main: main.c krun.h $(LIB_DIR)/libkrun_cdylib.so
	$(CC) -o $@ main.c -L$(LIB_DIR) -Wl,--no-as-needed -lkrun_cdylib

run: main
	LD_LIBRARY_PATH=$(LIB_DIR) ./main

clean:
	rm -f main krun.h
