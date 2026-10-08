.PHONY: all build test release lint

# Secondary goals after `test` / `release` (crate name or platform).
CMD  := $(firstword $(MAKECMDGOALS))
ARGS := $(wordlist 2,$(words $(MAKECMDGOALS)),$(MAKECMDGOALS))

ifneq (,$(filter $(CMD),test release))
$(ARGS):
	@:
endif

all: build

## Build the workspace (debug).
build:
	cargo build --workspace

## Run all workspace tests, or `make test <crate_name>...` for specific crates.
test:
ifeq ($(strip $(ARGS)),)
	cargo test --workspace
else
	cargo test $(foreach crate,$(ARGS),-p $(crate))
endif

## Release-build the browser for a platform: `make release <macos|linux|windows>`.
release:
ifeq ($(strip $(ARGS)),)
	$(error Usage: make release <platform> (macos|linux|windows))
endif
ifeq ($(ARGS),macos)
	cargo build --release --target aarch64-apple-darwin -p browser
else ifeq ($(ARGS),linux)
	cargo build --release --target x86_64-unknown-linux-gnu -p browser
else ifeq ($(ARGS),windows)
	cargo build --release --target x86_64-pc-windows-msvc -p browser
else
	$(error Unknown platform '$(ARGS)'. Expected macos, linux, or windows)
endif

## Format check + clippy (matches CI).
lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
