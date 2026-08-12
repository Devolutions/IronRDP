mod acceptor;
mod autodetect;
mod credential_validator;
mod fast_path;

// Pulled in from the crate itself: `ironrdp-server` sets `test = false`, so its
// inline unit tests only run when compiled as part of this test suite.
#[path = "../../../ironrdp-server/src/encoder/qoiz.rs"]
mod qoiz;
