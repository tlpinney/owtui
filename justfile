default: lint test

test:
    cargo nextest run --locked --status-level fail --final-status-level fail --failure-output final --success-output never

lint:
    cargo fmt --check
    cargo clippy --all-targets --locked -- -D warnings

build:
    cargo build --release --locked

install:
    cargo install --path . --locked

run:
    cargo run --locked

# Sign SHA256SUMS of a draft release made by CI and publish it.
sign-release tag:
    scripts/sign-release.sh {{tag}}
