pub(crate) struct ConfigParser(ironrdp_propertyset::PropertySet);

impl iron_remote_desktop::ConfigParser for ConfigParser {
    fn create(config: &str) -> Self {
        let mut properties = ironrdp_propertyset::PropertySet::new();

        if let Err(errors) = ironrdp_rdpfile::load(&mut properties, config) {
            for e in errors {
                error!("Error when reading configuration: {e}");
            }
        }

        Self(properties)
    }

    fn get_str(&self, key: &str) -> Option<String> {
        self.0.get::<&str>(key).map(|str| str.to_owned())
    }

    fn get_int(&self, key: &str) -> Option<i32> {
        self.0.get::<i32>(key)
    }
}
