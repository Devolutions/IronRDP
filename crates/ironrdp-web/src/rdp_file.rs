use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen]
pub struct RdpFile(ironrdp_propertyset::PropertySet);

#[wasm_bindgen]
impl RdpFile {
    #[wasm_bindgen(constructor)]
    pub fn create() -> Self {
        Self(ironrdp_propertyset::PropertySet::new())
    }

    pub fn parse(&mut self, config: &str) {
        let parse_result = ironrdp_rdpfile::parse(config);

        self.0 = parse_result.properties;

        for e in parse_result.errors {
            error!("Error when reading configuration: {e}");
        }
    }

    pub fn write(&self) -> String {
        ironrdp_rdpfile::write(&self.0)
    }

    #[wasm_bindgen(js_name = "insertStr")]
    pub fn insert_str(&mut self, key: String, value: &str) {
        self.0.insert(key, value);
    }

    #[wasm_bindgen(js_name = "insertInt")]
    pub fn insert_int(&mut self, key: String, value: i32) {
        self.0.insert(key, value);
    }

    #[wasm_bindgen(js_name = "getStr")]
    pub fn get_str(&self, key: &str) -> Option<String> {
        self.0.get::<&str>(key).map(|str| str.to_owned())
    }

    #[wasm_bindgen(js_name = "getInt")]
    pub fn get_int(&self, key: &str) -> Option<i32> {
        self.0.get::<i32>(key)
    }
}
