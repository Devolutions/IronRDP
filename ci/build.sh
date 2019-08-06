set -ex

cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings

cargo build
cargo build --release

cargo build --target wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release

cargo test
cargo test --release
