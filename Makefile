# Makefile for Code in Motion (cim) development workflow
# Default target: build, test, lint, format, and install

.PHONY: all build test clippy fmt install clean help

# Default target - runs the complete development workflow
all: build test clippy fmt install
	@echo "✓ All tasks completed successfully!"

# Build cim in release mode
build:
	@echo "Building cim..."
	cargo build --release

# Run all tests
test:
	@echo "Running tests..."
	cargo test --quiet

# Run clippy linter
clippy:
	@echo "Running clippy..."
	cargo clippy --all-targets --all-features -- -D warnings

# Format code
fmt:
	@echo "Formatting code..."
	cargo fmt

# Install cim CLI to $HOME/bin (depends on build)
install: build
	@echo "Installing cim to $$HOME/bin..."
	@mkdir -p $$HOME/bin
	@cp target/release/cim $$HOME/bin/
	@echo "✓ cim installed to $$HOME/bin/cim"

# Clean build artifacts
clean:
	@echo "Cleaning build artifacts..."
	cargo clean

# Display available targets
help:
	@echo "Available targets:"
	@echo "  make         - Run all (build, test, clippy, fmt, install)"
	@echo "  make all     - Same as 'make'"
	@echo "  make build   - Build cim in release mode"
	@echo "  make test    - Run all tests"
	@echo "  make clippy  - Run clippy linter"
	@echo "  make fmt     - Format code"
	@echo "  make install - Install cim CLI to $$HOME/bin"
	@echo "  make clean   - Clean build artifacts"
	@echo "  make help    - Show this help message"
