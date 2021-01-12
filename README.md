# Protobuf Rust Crate Generator
Tool for generating a Rust crate from one or multiple trees of protobuf files.
Protobuf code is generated using [PROST!](https://github.com/danburkert/prost) and gRCP using [Tonic](https://github.com/hyperium/tonic).

Generated code is structured in modules according to the protobuf package name.

## Build
```console
cargo build
```

## Example Usage
Generate a crate named `my-pb-crate` in direcotry `gen`  using protobuf files from the directories `proto/common`, `proto/internal` and `proto/external`:
```console
proto-crate-gen --output-dir gen --pkg-name my-pb-crate --pkg-version 0.2.1 proto/common proto/internal proto/external
```