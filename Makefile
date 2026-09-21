.DEFAULT_GOAL := help

SHELL  = /bin/bash
.SHELLFLAGS = -o pipefail -c

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
	cargo clean -p gittree -p gar -p gar-cli -p experiments
