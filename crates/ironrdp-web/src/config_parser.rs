use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen]
pub struct RdpConfigParser(ironrdp_propertyset::PropertySet);

#[wasm_bindgen]
impl RdpConfigParser {
    #[wasm_bindgen(constructor)]
    pub fn create(config: &str) -> Self {
        let mut properties = ironrdp_propertyset::PropertySet::new();

        if let Err(errors) = ironrdp_rdpfile::load(&mut properties, config) {
            for e in errors {
                error!("Error when reading configuration: {e}");
            }
        }

        Self(properties)
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
