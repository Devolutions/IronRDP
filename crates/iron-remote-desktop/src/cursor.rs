#[derive(Debug)]
pub enum CursorStyle {
    Default,
    Hidden,
    Url {
        data: String,
        hotspot_x: u16,
        hotspot_y: u16,
    },
}
