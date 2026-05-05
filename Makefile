.PHONY: test
test:
	cargo check --all-targets
	cargo test --verbose
	cargo clippy --all-targets --all-features
	cargo fmt --all --check
