#!/bin/sh

set -x

# Handy when debugging problems.
cargo --version
rustc --version

rustup component add clippy

# Failing the `cognitive-complexity' lint is allowed because the tests are too
# complex.
cargo clippy --all-targets --all-features -- \
	-D warnings \
	-A clippy::cognitive-complexity \
	-A clippy::needless_lifetimes # I disagree with this.
