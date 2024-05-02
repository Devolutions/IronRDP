static LOG_INITED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[diplomat::bridge]
pub mod ffi {
    use std::{fs::File, path::PathBuf};

    use super::LOG_INITED;
    use std::fs;

    #[diplomat::opaque]
    pub struct Log;

    impl Log {
        pub fn init_with_env() {
            if LOG_INITED.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            LOG_INITED.store(true, std::sync::atomic::Ordering::Relaxed);

            let filepath: PathBuf = std::env::var("IRONRDP_LOG_FILE")
                .unwrap_or_else(|_| "/tmp/ironrdp.log".to_owned())
                .try_into()
                .expect("Failed to parse log file path");

            fs::create_dir_all(filepath.parent().expect("Path has no parent")).expect("Failed to create log directory");
            let file = File::create(filepath).unwrap();
            tracing_subscriber::fmt::SubscriberBuilder::default()
                .with_env_filter(tracing_subscriber::EnvFilter::from_env("IRONRDP_LOG"))
                .with_writer(file)
                .init();
        }
    }
}
