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

    /// Initial bearing to `other` in degrees (0 = north, clockwise).
    /// Equirectangular approximation: fine for sim legs, not navigation.
    pub fn bearing_deg_to(&self, other: &Self) -> f64 {
        let d_lon = (other.longitude - self.longitude).to_radians();
        let d_lat = (other.latitude - self.latitude).to_radians();
        let x = d_lon * self.latitude.to_radians().cos();
        (x.atan2(d_lat).to_degrees() + 360.0) % 360.0
    }

    /// Dead reckoning: advance along `heading_deg` at `speed_kn` for
    /// `dt_secs`. Equirectangular approximation, matching `bearing_deg_to`.
    pub fn dead_reckon(&self, heading_deg: f32, speed_kn: f32, dt_secs: f64) -> Self {
        const R: f64 = 6_371_000.0;
        const KN_TO_MS: f64 = 0.514_444;
        let dist = speed_kn as f64 * KN_TO_MS * dt_secs;
        let hdg = (heading_deg as f64).to_radians();
        let dn = dist * hdg.cos();
        let de = dist * hdg.sin();
        Self {
            latitude: self.latitude + (dn / R).to_degrees(),
            longitude: self.longitude
                + (de / (R * self.latitude.to_radians().cos())).to_degrees(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dead_reckon_north_one_hour_at_60kn_is_one_degree() {
        // 60 kn = 1 nm/min = 1 degree latitude per hour, by definition.
        let a = GeoPosition { latitude: 0.0, longitude: 0.0 };
        let b = a.dead_reckon(0.0, 60.0, 3600.0);
        assert!((b.latitude - 1.0).abs() < 1e-3, "got {}", b.latitude);
        assert!(b.longitude.abs() < 1e-9);
    }

    #[test]
    fn bearing_cardinal_points() {
        let a = GeoPosition { latitude: 0.0, longitude: 0.0 };
        let n = GeoPosition { latitude: 1.0, longitude: 0.0 };
        let e = GeoPosition { latitude: 0.0, longitude: 1.0 };
        assert!((a.bearing_deg_to(&n) - 0.0).abs() < 1e-9);
        assert!((a.bearing_deg_to(&e) - 90.0).abs() < 1e-9);
    }

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
