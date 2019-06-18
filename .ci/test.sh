#!/bin/sh

set -x

# Handy when debugging problems.
cargo --version
rustc --version

if [ "$TRAVIS_OS_NAME" = "linux" ]; then
	# Enable IPv6 on Travis' Linux machines.
	sudo sh -c 'echo 0 > /proc/sys/net/ipv6/conf/all/disable_ipv6'
fi

cargo test --verbose
