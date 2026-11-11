.PHONY: all test

all:
	cargo build --release -p dfdu

test:
	@if cargo nextest --version >/dev/null 2>&1; then \
		cargo nextest run -p dfdu-session -p dfdu-tui; \
	else \
		cargo test -p dfdu-session -p dfdu-tui; \
	fi
