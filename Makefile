.PHONY: all test install clean-debug

all:
	cargo build --release -p dfdu

test:
	@if cargo nextest --version >/dev/null 2>&1; then \
		cargo nextest run -p dfdu-peer -p dfdu -p dfdu-session -p dfdu-tui; \
	else \
		cargo test -p dfdu-peer -p dfdu -p dfdu-session -p dfdu-tui; \
	fi

install: all
	mkdir -p "$(HOME)/.local/bin"
	# Replace the file, not its contents: macOS kills a binary rewritten in place
	# (stale code-signature cache, exit 137).
	rm -f "$(HOME)/.local/bin/dfdu"
	cp target/release/dfdu "$(HOME)/.local/bin/dfdu"

# Cargo never removes stale build outputs; this drops them all (release stays).
clean-debug:
	rm -rf target/debug
