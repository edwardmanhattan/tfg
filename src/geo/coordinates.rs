#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoPosition {
    pub latitude: f64,
    pub longitude: f64,
}

impl GeoPosition {
    /// Great-circle distance in meters (haversine, R = 6_371_000 m).
    pub fn distance_m(&self, other: &Self) -> f64 {
        const R: f64 = 6_371_000.0;
        let d_lat = (other.latitude - self.latitude).to_radians();
        let d_lon = (other.longitude - self.longitude).to_radians();
        let a = (d_lat / 2.0).sin().powi(2)
            + self.latitude.to_radians().cos()
                * other.latitude.to_radians().cos()
                * (d_lon / 2.0).sin().powi(2);
        2.0 * R * a.sqrt().asin()
    }

    /// Linear interpolation between two positions. `t` is clamped to [0, 1].
    pub fn lerp(&self, other: &Self, t: f64) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            latitude: self.latitude + (other.latitude - self.latitude) * t,
            longitude: self.longitude + (other.longitude - self.longitude) * t,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haversine_hamburg_harbor_is_about_9km() {
        let a = GeoPosition { latitude: 53.5413, longitude: 9.9842 };
        let b = GeoPosition { latitude: 53.50, longitude: 9.90 };
        let d = a.distance_m(&b);
        assert!((7_000.0..7_500.0).contains(&d), "got {d}");
    }

    #[test]
    fn lerp_midpoint_and_clamp() {
        let a = GeoPosition { latitude: 0.0, longitude: 0.0 };
        let b = GeoPosition { latitude: 10.0, longitude: 20.0 };
        assert_eq!(a.lerp(&b, 0.5), GeoPosition { latitude: 5.0, longitude: 10.0 });
        assert_eq!(a.lerp(&b, 2.0), b);
        assert_eq!(a.lerp(&b, -1.0), a);
    }
}
