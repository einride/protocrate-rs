# all: all tasks required for a complete build
.PHONY: all
all: \
	build \
	fmt-check \
	clippy \
	test

export PROTOC := protoc
export PROTOC_INCLUDE := .

# GIT_BUILD_REV will be embeded in the plugin as build revision at buildtime
SHORT_SHA ?= $(shell git describe --always --dirty)

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
test:
	RUST_BACKTRACE=1 cargo test --all

.PHONY: build/release
build/release:
	cargo build --all  --all-targets --release
	cp target/release/proto-crate-gen target/release/${SHORT_SHA}_proto-crate-gen