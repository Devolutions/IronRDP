pub trait ConfigParser {
    #[must_use]
    fn create(config: &str) -> Self;

    #[must_use]
    fn get_str(&self, key: &str) -> Option<String>;

    #[must_use]
    fn get_int(&self, key: &str) -> Option<i32>;
}
