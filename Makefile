# TERYON developer shortcuts. Run `make help`.
.DEFAULT_GOAL := help
FEATURES ?= desktop
HOST ?= local

.PHONY: help run tui desktop build release app install check test fmt lint clean stop

help: ## Show this help
	@awk 'BEGIN{FS=":.*## "} /^[a-z-]+:.*## /{printf "  make %-9s %s\n",$$1,$$2}' $(MAKEFILE_LIST)
	@echo "  vars: HOST=$(HOST) FEATURES=$(FEATURES)"

run: ## Run the desktop app on HOST (default local)
	cargo run --release --features $(FEATURES) -- desktop $(HOST)

desktop: run ## Alias of run

tui: ## Run the terminal UI on HOST
	cargo run --release -- connect $(HOST)

build: ## Debug build with the desktop feature
	cargo build --features $(FEATURES)

release: ## Optimized build (target/release/teryon)
	cargo build --release --features $(FEATURES) --locked

app: ## Build dist/TERYON.app (macOS bundle, unsigned)
	sh scripts/bundle-macos.sh

install: ## Install teryon into ~/.cargo/bin
	cargo install --path . --features $(FEATURES) --locked --force

check: fmt lint test ## fmt + clippy + tests (what CI runs)

fmt: ## Check formatting
	cargo fmt --all --check

lint: ## Clippy with warnings as errors
	cargo clippy --all-targets --all-features --locked -- -D warnings

test: ## Run all tests
	cargo test --all-features --locked

stop: ## Stop the local daemon
	-cargo run -q --release --features $(FEATURES) -- stop local --force

clean: ## Remove build output and dist/
	cargo clean
	rm -rf dist
