#!/usr/bin/env sh

set -x

# Handy when debugging problems.
cargo --version
rustc --version

rustup component add rustfmt

cargo fmt --all -- --check
