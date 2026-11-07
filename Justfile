# Default recipe
default:
    just --list

# Build the project
build:
    cargo build --release

# Run tests
test:
    cargo test --all

# Format and lint
check:
    cargo fmt --check
    cargo clippy -- -D warnings

# Build and immediately run
run *args:
    cargo run -- {{args}}