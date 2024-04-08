#[diplomat::bridge]
pub mod ffi {
    #[diplomat::opaque]
    pub struct Database(pub ironrdp::input::Database);

    impl Database {
        pub fn new() -> Box<Database> {
            Box::new(Database(ironrdp::input::Database::new()))
        }
    }
}
