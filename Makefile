.DEFAULT_GOAL := help

SHELL  = /bin/bash
.SHELLFLAGS = -o pipefail -c

MADE ?= $(HOME)/made

.PHONY: all
all: fmt build test ## Everything that's a must before committing: fmt, build, test.

.PHONY: help
help: ## Print info about all commands
	@echo "Commands:"
	@echo
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "    \033[01;32m%-20s\033[0m %s\n", $$1, $$2}'

.PHONY: setup
setup: ## One-time dev env setup: activate git hooks.
	git config core.hooksPath hack/git-hooks

.PHONY: fmt
fmt: ## Format Rust source in place.
	cargo fmt --all

.PHONY: check
check: ## Verify-only and fast; run by the pre-commit hook on every commit.
	cargo fmt --all --check

.PHONY: build
build: ## Build all crates (dev profile).
	cargo build --workspace

.PHONY: test
test: ## Run all tests.
	cargo test --workspace

.PHONY: clean
clean: ## Remove this workspace's build outputs (scoped: the target dir may be shared via CARGO_TARGET_DIR).
	cargo clean --workspace

# Static linking against the host's own libc, so any toolchain that builds gar at all can build this:
# no extra std components, no extra tools, and (as everywhere in gar) no C is compiled.
# The explicit --target is load-bearing even though it names the host:
# it keeps RUSTFLAGS off build scripts and proc macros, which must stay host-dynamic.
.PHONY: static
static: ## Build a statically linked release binary.
	t=$$(rustc -vV | sed -n 's/^host: //p') \
	&& RUSTFLAGS="-C target-feature=+crt-static" cargo build --release --target "$$t" -p gar-cli \
	&& echo "binary: $${CARGO_TARGET_DIR:-target}/$$t/release/gar"

# `--locked` because `cargo install --path` otherwise ignores Cargo.lock and resolves everything afresh.
.PHONY: install
install: ## Install binaries into $(MADE)/gar/bin/.
	cargo install --locked --no-track --force --path crates/gar-cli --root "$(MADE)/gar"
