.PHONY: all build run test release lint

# Secondary goals after `run`, `test`, or `release` (crate name or platform).
CMD  := $(firstword $(MAKECMDGOALS))
ARGS := $(wordlist 2,$(words $(MAKECMDGOALS)),$(MAKECMDGOALS))

ifneq (,$(filter $(CMD),run test release))
$(ARGS):
	@:
endif

all: build

## Build the workspace (debug).
build:
	cargo build --workspace

# `make run LOG=debug` (or any --log-level spec, e.g. LOG=warn,net=trace)
# forwards a log level to the program being run.
RUN_ARGS := $(if $(LOG),-- --log-level $(LOG))

## Run a platform shell: `make run shell` for the browser, or `make run js`
## for the interactive JavaScript engine shell. Add `LOG=<spec>` to set the
## log level.
run:
ifeq ($(ARGS),js)
	cargo run -p browser-js $(RUN_ARGS)
else ifeq ($(ARGS),shell)
	cargo run -p browser $(RUN_ARGS)
else ifeq ($(OS),Windows_NT)
	cargo run -p byui-browser-windows $(RUN_ARGS)
else ifneq (,$(filter Darwin darwin,$(shell uname -s)))
	cargo run -p browser-macos $(RUN_ARGS)
else ifneq (,$(filter Linux linux,$(shell uname -s)))
	cargo run -p browser-linux $(RUN_ARGS)
else ifneq (,$(filter MINGW% MSYS% CYGWIN%,$(shell uname -s)))
	cargo run -p byui-browser-windows $(RUN_ARGS)
else
	$(error Unsupported operating system '$(shell uname -s)')
endif

## Run all workspace tests, or `make test <crate_name>...` for specific crates.
test:
ifeq ($(strip $(ARGS)),)
	cargo test --workspace
else
	cargo test $(foreach crate,$(ARGS),-p $(crate))
endif

## Release-build the native shell: `make release <macos|linux|windows>`.
## The release workflow builds these same packages. Keep the two in sync.
release:
ifeq ($(strip $(ARGS)),)
	$(error Usage: make release <platform> (macos|linux|windows))
endif
ifeq ($(ARGS),macos)
	cargo build --locked --release --target aarch64-apple-darwin -p browser-macos
else ifeq ($(ARGS),linux)
	cargo build --locked --release --target x86_64-unknown-linux-gnu -p browser-linux
else ifeq ($(ARGS),windows)
	cargo build --locked --release --target x86_64-pc-windows-msvc -p byui-browser-windows
else
	$(error Unknown platform '$(ARGS)'. Expected macos, linux, or windows)
endif

## Format check + clippy (matches CI).
lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
