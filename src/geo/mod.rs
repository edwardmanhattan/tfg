#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoPosition {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenPosition {
    pub x: f32,
    pub y: f32,
}

impl GeoPosition {
    pub fn new(latitude: f64, longitude: f64) -> Option<Self> {
        if latitude < -90.0 || latitude > 90.0 || longitude < -180.0 || longitude > 180.0 {
            return None;
        }

        Some(Self {
            latitude,
            longitude,
        })
    }
}
