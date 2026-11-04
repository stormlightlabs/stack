# The gates a pull request must pass. CI runs `just check`.
check:
    cargo fmt --all --check
    cargo clippy --locked --workspace --all-targets -- -D warnings
    cargo test --locked --workspace
    fab check
    arcl workspace import --quiet
    arcl check

fmt:
    cargo fmt --all
