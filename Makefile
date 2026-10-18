.PHONY: build desktop check test package
build:
	cargo build --release --locked
desktop:
	cargo build --release --features desktop --locked
check:
	cargo fmt --all --check
	cargo clippy --all-targets --all-features --locked -- -D warnings
test:
	cargo test --all-targets --all-features --locked
package:
	./scripts/package.sh
