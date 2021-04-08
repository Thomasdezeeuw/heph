#!/usr/bin/env bash

set -eu

# Install nightly compiler.
rustup toolchain install nightly
rustup default nightly

# Show versions, useful for debugging later.
rustc -Vv
cargo -V
