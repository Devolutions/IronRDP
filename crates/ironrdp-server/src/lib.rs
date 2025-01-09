#![doc = include_str!("../README.md")]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![allow(clippy::arithmetic_side_effects)] // TODO: should we enable this lint back?

pub use {tokio, tokio_rustls};

#[macro_use]
extern crate tracing;

mod builder;
mod capabilities;
mod clipboard;
mod display;
mod encoder;
mod handler;
#[cfg(feature = "helper")]
mod helper;
mod server;
mod sound;

pub use clipboard::*;
pub use display::*;
pub use handler::*;
#[cfg(feature = "helper")]
pub use helper::*;
pub use server::*;
pub use sound::*;

#[cfg(feature = "__bench")]
pub mod bench {
    pub mod encoder {
        pub mod rfx {
            pub use crate::encoder::rfx::bench::{rfx_enc, rfx_enc_tile};
        }
    }
}

#[macro_export]
macro_rules! time_warn {
    ($context:expr, $threshold_ms:expr, $op:expr) => {{
        #[cold]
        fn warn_log(context: &str, duration: u128) {
            use ::core::sync::atomic::AtomicUsize;

            static COUNT: AtomicUsize = AtomicUsize::new(0);
            let current_count = COUNT.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed);
            if current_count < 50 || current_count % 100 == 0 {
                ::tracing::warn!("{context} took {duration} ms! (count: {current_count})");
            }
        }

        let start = std::time::Instant::now();
        let result = $op;
        let duration = start.elapsed().as_millis();
        if duration > $threshold_ms {
            warn_log($context, duration);
        }
        result
    }};
}
