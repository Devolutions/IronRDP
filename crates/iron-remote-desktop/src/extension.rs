use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;

#[macro_export]
macro_rules! extension_match {
    ( @ $jsval:expr, $value:ident, String, $operation:block ) => {{
        if let Some($value) = $jsval.as_string() {
            $operation
        } else {
            warn!("Unexpected value for extension {}", stringify!($ident));
        }
    }};
    ( @ $jsval:expr, $value:ident, f64, $operation:block ) => {{
        if let Some($value) = $jsval.as_f64() {
            $operation
        } else {
            warn!("Unexpected value for extension {}", stringify!($ident));
        }
    }};
    ( @ $jsval:expr, $value:ident, bool, $operation:block ) => {{
        if let Some($value) = $jsval.as_bool() {
            $operation
        } else {
            warn!("Unexpected value for extension {}", stringify!($ident));
        }
    }};
    ( @ $jsval:expr, $value:ident, JsValue, $operation:block ) => {{
        let $value = $jsval;
        $operation
    }};

    ( match $ext:ident ; $( | $value:ident : $ty:ident | $operation:block ; )* ) => {
        let ident = $ext.ident();

        match ident {
            $( stringify!($value) => $crate::extension_match!( @ $ext.into_value(), $value, $ty, $operation ), )*
            unknown_extension => ::tracing::warn!("Unknown extension: {unknown_extension}"),
        }
    };
}

#[wasm_bindgen]
pub struct Extension {
    ident: String,
    value: JsValue,
}

#[wasm_bindgen]
impl Extension {
    #[wasm_bindgen(constructor)]
    pub fn create(ident: String, value: JsValue) -> Self {
        Self { ident, value }
    }
}

impl Extension {
    pub fn ident(&self) -> &str {
        self.ident.as_str()
    }

    pub fn value(&self) -> &JsValue {
        &self.value
    }

    pub fn into_value(self) -> JsValue {
        self.value
    }
}
