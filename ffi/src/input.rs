#[diplomat::bridge]
pub mod ffi {
    #[diplomat::opaque]
    pub struct InputDatabase(pub ironrdp::input::Database);

    impl InputDatabase {
        pub fn new() -> Box<InputDatabase> {
            Box::new(InputDatabase(ironrdp::input::Database::new()))
        }
    }
}
