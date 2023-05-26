include Makefile.include

# Crates in this repo.
CRATES := ./ inbox rt remote http
# Target that run the target in all $CRATES.
TARGETS := test_all test_sanitizers_all test_sanitizer_all check_all clippy_all

# This little construct simply runs the target ($MAKECMDGOALS) for all crates
# $CRATES.
$(TARGETS): $(CRATES)
$(CRATES):
	@$(MAKE) -C $@ $(patsubst %_all,%,$(MAKECMDGOALS))

lint_all: clippy_all

doc_all:
	cargo doc --all-features --workspace

doc_all_private:
	cargo doc --all-features --workspace --document-private-items

clean_all:
	cargo clean

.PHONY: $(TARGETS) $(CRATES) test_all test_sanitizers_all test_sanitizer_all check_all clippy_all lint_all doc_all doc_private_all clean_all
