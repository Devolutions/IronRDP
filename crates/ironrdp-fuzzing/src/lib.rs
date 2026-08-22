// No need to be as strict as in production libraries
#![allow(clippy::arithmetic_side_effects)]
// Panicking is the documented way an oracle reports a bug to the fuzzing
// engine, so neither the panic itself nor a `# Panics` section on every oracle
// carries information here.
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]

pub mod generators;
pub mod oracles;
