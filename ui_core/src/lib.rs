pub mod draw;
pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;
pub type Result<T> = std::result::Result<T, Error>;
pub mod renderer;
mod surface;
