# Set by `rustup run`, or we get it ourselves.
# Example value: `nightly-x86_64-apple-darwin`.
RUSTUP_TOOLCHAIN ?= "$(shell rustup show active-toolchain | cut -d' ' -f1)"
# Architecture target. Example value: `x86_64-apple-darwin`.
RUSTUP_TARGET    ?= "$(shell echo $(RUSTUP_TOOLCHAIN) | cut -d'-' -f2,3,4)"
# Location of LLVM tools, as install by `install_llvm_tools`.
LLVM_BIN         ?= "$(shell rustc --print sysroot)/lib/rustlib/$(RUSTUP_TARGET)/bin"
# Where we put the coverage output.
COVERAGE_OUTPUT  ?= "./target/coverage"
# Targets available via Rustup that are supported.
TARGETS ?= "x86_64-apple-darwin" "x86_64-unknown-linux-gnu" "x86_64-unknown-freebsd"

test:
	cargo test --all-features

check_all_targets: $(TARGETS)
$(TARGETS):
	cargo check --all-features --all-targets --target $@

# NOTE: when using this command you might want to change the `test` target to
# only run a subset of the tests you're actively working on.
dev:
	find src/ tests/ examples/ Makefile Cargo.toml | entr -d -c $(MAKE) test

clippy: lint
lint:
	cargo clippy --all-targets --all-features -- \
		--warn warnings \
		--warn clippy::correctness \
		--warn clippy::style \
		--warn clippy::complexity \
		--warn clippy::perf \
		-D warnings \
		-A clippy::cognitive-complexity \
		-A clippy::needless_lifetimes \
		-A clippy::unnecessary-wraps

install_clippy:
	rustup component add clippy

coverage:
	rm -rf "$(COVERAGE_OUTPUT)"
	@# Run the tests with the LLVM instrumentation.
	RUSTFLAGS="$(RUSTFLAGS) -Zinstrument-coverage" \
		LLVM_PROFILE_FILE="$(COVERAGE_OUTPUT)/tests.%p.profraw" \
		$(MAKE) --always-make test
	@# Merge all coverage data into a single profile.
	"$(LLVM_BIN)/llvm-profdata" merge \
		--output "$(COVERAGE_OUTPUT)/tests.profdata" \
		"$(COVERAGE_OUTPUT)"/tests.*.profraw
	@# Generate a HTML report for the coverage, excluding all files not in `src/`.
	find target/debug/deps -perm -111 -type f -maxdepth 1 | xargs printf -- "--object '%s' " | xargs  \
		"$(LLVM_BIN)/llvm-cov" show \
		--show-instantiations=false \
		--show-expansions \
		--ignore-filename-regex "^[^src]" \
		--format html \
		--output-dir "$(COVERAGE_OUTPUT)/report" \
		--instr-profile "$(COVERAGE_OUTPUT)/tests.profdata"
	open "$(COVERAGE_OUTPUT)/report/index.html"

install_coverage: install_llvm_tools

install_llvm_tools:
	rustup component add llvm-tools-preview

clean:
	cargo clean

.PHONY: test dev clippy lint install_clippy coverage install_coverage install_llvm_tools clean
