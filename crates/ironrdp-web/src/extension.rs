use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;
use wasm_bindgen_derive::try_from_js_option;

#[derive(wasm_bindgen_derive::TryFromJsValue)]
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct DisplayControl(bool);

#[wasm_bindgen]
impl DisplayControl {
    pub fn new(value: bool) -> Self {
        Self(value)
    }
}

impl DisplayControl {
    pub fn into_inner(self) -> bool {
        self.0
    }
}

#[derive(wasm_bindgen_derive::TryFromJsValue)]
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct Pcb(String);

#[wasm_bindgen]
impl Pcb {
    pub fn new(value: String) -> Self {
        Self(value)
    }
}

impl Pcb {
    pub fn into_inner(self) -> String {
        self.0
    }
}

#[derive(wasm_bindgen_derive::TryFromJsValue)]
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct KdcProxyUrl(String);

#[wasm_bindgen]
impl KdcProxyUrl {
    pub fn new(value: String) -> Self {
        Self(value)
    }
}

impl KdcProxyUrl {
    pub fn into_inner(self) -> String {
        self.0
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Extension {
    DisplayControl(bool),
    Pcb(String),
    KdcProxyUrl(String),
}

impl Extension {
    pub(crate) fn try_from_js_value(value: JsValue) -> Result<Self, anyhow::Error> {
        if let Ok(Some(display_control)) = try_from_js_option::<DisplayControl>(value.clone()) {
            Ok(Self::DisplayControl(display_control.into_inner()))
        } else if let Ok(Some(pcb)) = try_from_js_option::<Pcb>(value.clone()) {
            Ok(Self::Pcb(pcb.into_inner()))
        } else if let Ok(Some(kdc)) = try_from_js_option::<KdcProxyUrl>(value) {
            Ok(Self::KdcProxyUrl(kdc.into_inner()))
        } else {
            anyhow::bail!("provided value is not a supported extension")
        }
    }
}
