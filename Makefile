# all: all tasks required for a complete build
.PHONY: all
all: \
	build \
	fmt-check \
	clippy \
	test

export PROTOC := protoc
export PROTOC_INCLUDE := .

.PHONY: clean
clean:
	rm -fR target

.PHONY: build
build:
	cargo build --all --all-targets

.PHONY: clippy
clippy:
	cargo clippy --all -- -D warnings

.PHONY: fmt-check
fmt-check:
	cargo fmt --all -- --check

.PHONY: test
test: build
	RUST_BACKTRACE=1 cargo test --all
	cargo run -- test --output-dir=gen --pkg-name=test
	cd gen && cargo build
