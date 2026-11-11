.PHONY: all test clean-debug

all:
	cargo build --release -p dfdu

test:
	@if cargo nextest --version >/dev/null 2>&1; then \
		cargo nextest run -p dfdu-session -p dfdu-tui; \
	else \
		cargo test -p dfdu-session -p dfdu-tui; \
	fi

# Cargo never removes stale build outputs; this drops them all (release stays).
clean-debug:
	rm -rf target/debug
