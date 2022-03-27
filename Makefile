# Test options.
TEST_OPTS         = -- --quiet -Z unstable-options --shuffle
# Set by `rustup run`, or we get it ourselves.
# Example value: `nightly-x86_64-apple-darwin`.
RUSTUP_TOOLCHAIN ?= $(shell rustup show active-toolchain | cut -d' ' -f1)
# Architecture target. Example value: `x86_64-apple-darwin`.
RUSTUP_TARGET    ?= $(shell echo $(RUSTUP_TOOLCHAIN) | cut -d'-' -f2,3,4,5)
# Location of LLVM tools, as install by `install_llvm_tools`.
LLVM_BIN         ?= $(shell rustc --print sysroot)/lib/rustlib/$(RUSTUP_TARGET)/bin
# To support `coverage` in workspaces we need to handle the single target
# directory.
# Absolute path to the root of the workspace.
WORKSPACE        = $(shell cargo locate-project --message-format plain --workspace | xargs dirname)
# Target directory inside the workspace (and all crates within it).
TARGET_DIR       = $(WORKSPACE)/target
# Output directory for the coverage data.
COVERAGE_OUTPUT  = $(TARGET_DIR)/coverage
# Targets available via Rustup that are supported.
TARGETS ?= x86_64-apple-darwin x86_64-unknown-linux-gnu x86_64-unknown-freebsd
# Command to run in `dev` target, e.g. `make RUN=check dev`.
RUN ?= test

test:
	cargo test --features runtime,test $(TEST_OPTS)

test_all:
	cargo hack test --workspace --exclude benches --all-targets --feature-powerset --skip crossbeam-channel,libc,mio,mio-signals,socket2,getrandom $(TEST_OPTS)

# NOTE: Keep `RUSTFLAGS` and `RUSTDOCFLAGS` in sync to ensure the doc tests
# compile correctly.
test_sanitiser:
	@if [ -z $${SAN+x} ]; then echo "Required '\$$SAN' variable is not set" 1>&2; exit 1; fi
	RUSTFLAGS="-Z sanitizer=$$SAN -Z sanitizer-memory-track-origins" \
	RUSTDOCFLAGS="-Z sanitizer=$$SAN -Z sanitizer-memory-track-origins" \
	cargo test -Z build-std --all-features --workspace --target $(RUSTUP_TARGET)

check:
	cargo check --all-features --all-targets

check_all:
	cargo hack check --workspace --all-targets --feature-powerset --skip crossbeam-channel,libc,mio,mio-signals,socket2,getrandom

check_all_targets: $(TARGETS)
$(TARGETS):
	cargo check --all-features --workspace --all-targets --target $@

# NOTE: when using this command you might want to change the `test` target to
# only run a subset of the tests you're actively working on.
dev:
	find src/ tests/ examples/ Makefile Cargo.toml | entr -d -c $(MAKE) $(RUN)

# Reasons to allow lints:
# `cargo-common-metadata`: for `benches` and `tools`.
# `equatable-if-let`: bad lint.
# `match-bool`, `single-match-else`: often less lines of code and I find that
# use `match` generally strictly better then `if`s.
# `missing-const-for-fn`: See https://github.com/rust-lang/rust-clippy/issues/4979.
# `module-name-repetitions`: we re-export various names.
# `needless-lifetimes`: lifetime serves as documentation.
# `option-if-let-else`: not idiomatic at all.
# `use-self`: this is a bad lint.
#
# # Could fix these later
# `enum-glob-use`: used in enum errors.
# `missing-errors-doc`, `missing-panics-doc`: don't want to do this.
# `multiple-crate-versions`: caused by getrandom, pr to fix:
# <https://github.com/rust-random/getrandom/pull/253>.
# Too many warnings:
#  * `must-use-candidate`.
#  * `ptr-as-ptr`.
#  * `redundant-pub-crate`.
#  * `semicolon-if-nothing-returned`.
#  * `shadow-unrelated`.
clippy: lint
lint:
	cargo clippy --all-features --workspace -- \
		--deny clippy::all \
		--deny clippy::correctness \
		--deny clippy::style \
		--deny clippy::complexity \
		--deny clippy::perf \
		--deny clippy::pedantic \
		--deny clippy::nursery \
		--deny clippy::cargo \
		--allow clippy::cargo-common-metadata \
		--allow clippy::enum-glob-use \
		--allow clippy::equatable-if-let \
		--allow clippy::match-bool \
		--allow clippy::missing-const-for-fn \
		--allow clippy::missing-errors-doc \
		--allow clippy::missing-panics-doc \
		--allow clippy::module-name-repetitions \
		--allow clippy::must-use-candidate \
		--allow clippy::needless-lifetimes \
		--allow clippy::option-if-let-else \
		--allow clippy::ptr-as-ptr \
		--allow clippy::redundant-pub-crate \
		--allow clippy::semicolon-if-nothing-returned \
		--allow clippy::shadow-unrelated \
		--allow clippy::single-match-else \
		--allow clippy::multiple-crate-versions \
		--allow clippy::use-self

install_clippy:
	rustup component add clippy

coverage:
	rm -rf "$(COVERAGE_OUTPUT)"
	@# Run the tests with the LLVM instrumentation.
	RUSTFLAGS="$(RUSTFLAGS) -Zinstrument-coverage" \
		LLVM_PROFILE_FILE="$(COVERAGE_OUTPUT)/tests.%m.profraw" \
		$(MAKE) --always-make test
	@# Merge all coverage data into a single profile.
	"$(LLVM_BIN)/llvm-profdata" merge \
		--output "$(COVERAGE_OUTPUT)/tests.profdata" \
		"$(COVERAGE_OUTPUT)"/tests.*.profraw
	@# Generate a HTML report for the coverage, excluding all files not in `src/`.
	cd "$(WORKSPACE)" && \
		find $(TARGET_DIR)/debug/deps -perm -111 -type f -maxdepth 1 | xargs printf -- "--object '%s' " | xargs  \
		"$(LLVM_BIN)/llvm-cov" show \
		--show-instantiations=false \
		--show-expansions \
		--ignore-filename-regex=".cargo\/registry" \
		--ignore-filename-regex=".cargo\/git" \
		--ignore-filename-regex=".rustup" \
		--ignore-filename-regex="tests\/" \
		--ignore-filename-regex="tests.rs$$" \
		--format=html \
		--output-dir "$(COVERAGE_OUTPUT)/report" \
		--instr-profile="$(COVERAGE_OUTPUT)/tests.profdata"
	open "$(COVERAGE_OUTPUT)/report/index.html"

install_coverage: install_llvm_tools

install_llvm_tools:
	rustup component add llvm-tools-preview

doc:
	cargo doc --all-features --workspace

doc_private:
	cargo doc --all-features --workspace --document-private-items

clean:
	cargo clean

.PHONY: test test_all test_sanitiser check check_all check_all_targets dev clippy lint install_clippy coverage install_coverage install_llvm_tools doc doc_private clean
