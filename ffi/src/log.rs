use core::error::Error;
use std::sync::Once;

static INIT_LOG: Once = Once::new();

const IRONRDP_LOG_PATH: &str = "IRONRDP_LOG_PATH";
const IRONRDP_LOG: &str = "IRONRDP_LOG";

#[diplomat::bridge]
pub mod ffi {
    use super::{setup_logging, INIT_LOG, IRONRDP_LOG_PATH};

    #[diplomat::opaque]
    pub struct Log;

    impl Log {
        pub fn init_with_env() {
            INIT_LOG.call_once(|| {
                let log_file = std::env::var(IRONRDP_LOG_PATH).ok();
                let log_file = log_file.as_deref();
                setup_logging(log_file).expect("Failed to setup logging");
            });
        }
    }
}

fn setup_logging(log_file_path: Option<&str>) -> Result<(), Box<dyn Error>> {
    use std::fs::{create_dir_all, OpenOptions};
    use std::path::PathBuf;

    use tracing::metadata::LevelFilter;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::EnvFilter;

    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::WARN.into())
        .with_env_var(IRONRDP_LOG)
        .from_env_lossy();

    if let Some(log_file_path) = log_file_path {
        let path = PathBuf::from(log_file_path);
        if let Some(parent) = path.parent() {
            create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(log_file_path)?;

        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(file)
            .compact();
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .try_init()?;
    } else {
        let fmt_layer = tracing_subscriber::fmt::layer()
            .compact()
            .with_file(true)
            .with_line_number(true)
            .with_thread_ids(true)
            .with_target(false);
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .try_init()?;
    };

    Ok(())
}
