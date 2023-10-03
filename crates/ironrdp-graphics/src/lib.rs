#![allow(clippy::arithmetic_side_effects)] // FIXME: remove
#![allow(clippy::cast_lossless)] // FIXME: remove
#![allow(clippy::cast_possible_truncation)] // FIXME: remove
#![allow(clippy::cast_possible_wrap)] // FIXME: remove
#![allow(clippy::cast_sign_loss)] // FIXME: remove

pub mod color_conversion;
pub mod dwt;
pub mod image_processing;
pub mod pointer;
pub mod quantization;
pub mod rdp6;
pub mod rectangle_processing;
pub mod rle;
pub mod rlgr;
pub mod subband_reconstruction;
pub mod zgfx;

mod utils;
