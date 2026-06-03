.PHONY: all build release demo test check fmt clean

all: release demo

build:
	cargo build

release:
	cargo build --release

demo: doc/demo.gif

doc/demo.gif: doc/demo.tape release
	vhs doc/demo.tape

test:
	cargo test

check:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test

fmt:
	cargo fmt

clean:
	cargo clean
	rm -f doc/demo.gif
