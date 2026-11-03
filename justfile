# The gates a pull request must pass. CI runs `just check`.
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    fab check
    arcl check

fmt:
    cargo fmt --all
