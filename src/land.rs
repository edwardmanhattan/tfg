//! Land/water test for sim collision (ticket #22).
//!
//! Backed by Natural Earth 50m land polygons (GeoJSON, committed at
//! `assets/ne_50m_land.json`, public domain). Loaded once; every sim tick
//! asks [`Land::is_water`] before advancing a ship, and order commit asks
//! [`Land::is_water`] on the waypoint. Antimeridian-safe: polygon edges
//! are unwrapped so a ring crossing ±180° is tested as one continuous
//! strip (shortest path).

use serde_json::Value;

use crate::geo::coordinates::GeoPosition;

/// Step length for path sampling, in meters. A ship advances at most
/// `speed × game_dt` per tick; the sim checks the destination and the
/// samples along the way, so this bounds undetected hops over thin land.
pub const PATH_SAMPLE_M: f64 = 200.0;

/// One polygon ring in unwrapped longitudes (degrees). Land shapes from
/// Natural Earth are small enough that per-ring brute force is fine.
#[derive(Debug, Clone)]
struct Ring {
    pts: Vec<(f64, f64)>,
}

impl Ring {
    fn parse(coords: &[Value]) -> Option<Self> {
        let mut pts = Vec::with_capacity(coords.len());
        for c in coords {
            let pair = c.as_array()?;
            let lon = pair.first()?.as_f64()?;
            let lat = pair.get(1)?.as_f64()?;
            pts.push((lon, lat));
        }
        if pts.len() < 4 {
            return None;
        }
        // Unwrap: walk the ring, shifting each edge's longitudes by ±360
        // so consecutive points stay within 180° of each other. A ring
        // crossing ±180° (Chukotka, Fiji, Antarctica) then tests as one
        // continuous strip instead of wrapping the wrong way round.
        let mut unwrapped: Vec<(f64, f64)> = Vec::with_capacity(pts.len());
        let mut prev = pts[0];
        unwrapped.push(prev);
        for &(mut lon, lat) in pts.iter().skip(1) {
            while lon - prev.0 > 180.0 {
                lon -= 360.0;
            }
            while lon - prev.0 < -180.0 {
                lon += 360.0;
            }
            unwrapped.push((lon, lat));
            prev = (lon, lat);
        }
        Some(Self { pts: unwrapped })
    }

    /// Ray-casting point-in-ring, for the query longitude `lon` (any
    /// branch: all ring longitudes are consistent within 360°).
    fn contains(&self, lon: f64, lat: f64) -> bool {
        let mut inside = false;
        let n = self.pts.len();
        let mut j = n - 1;
        for i in 0..n {
            let (xi, yi) = self.pts[i];
            let (xj, yj) = self.pts[j];
            if ((yi > lat) != (yj > lat))
                && lon < (xj - xi) * (lat - yi) / (yj - yi) + xi
            {
                inside = !inside;
            }
            j = i;
        }
        inside
    }
}

/// Land polygons + the point/path tests built on them.
#[derive(Debug, Clone, Default)]
pub struct Land {
    /// Outer rings only: NE 50m `land` has no lakes, and holes this
    /// ticket does not need. Points in a Caspian-scale lake would be
    /// misjudged as land — acceptable for a sandbox.
    rings: Vec<Ring>,
}

impl Land {
    /// Parse a Natural Earth land FeatureCollection (GeoJSON).
    pub fn from_geojson(text: &str) -> Result<Self, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let features = v["features"]
            .as_array()
            .ok_or("not a FeatureCollection")?;
        let mut rings = Vec::new();
        for f in features {
            let geom = &f["geometry"];
            match geom["type"].as_str() {
                Some("Polygon") => {
                    for ring in geom["coordinates"].as_array().into_iter().flatten() {
                        if let Some(r) = Ring::parse(ring.as_array().expect("ring array")) {
                            rings.push(r);
                        }
                    }
                }
                Some("MultiPolygon") => {
                    for poly in geom["coordinates"].as_array().into_iter().flatten() {
                        for ring in poly.as_array().into_iter().flatten() {
                            if let Some(r) = Ring::parse(ring.as_array().expect("ring array")) {
                                rings.push(r);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if rings.is_empty() {
            return Err("no land polygons parsed".into());
        }
        Ok(Self { rings })
    }

    /// Load the Natural Earth data embedded in the executable.
    pub fn from_default_asset() -> Result<Self, String> {
        Self::from_geojson(crate::assets::LAND_JSON)
            .map_err(|e| format!("embedded land asset invalid: {e}"))
    }

    /// True when the point is in the sea (or any water NE 50m models).
    pub fn is_water(&self, p: &GeoPosition) -> bool {
        !self.rings.iter().any(|r| r.contains(p.longitude, p.latitude))
    }

    /// True when the straight path `a -> b` stays in water. Samples the
    /// great-circle-ish segment every [`PATH_SAMPLE_M`]; checks both ends
    /// plus interior samples.
    pub fn path_is_water(&self, a: &GeoPosition, b: &GeoPosition) -> bool {
        if !self.is_water(a) || !self.is_water(b) {
            return false;
        }
        let dist = a.distance_m(b);
        if dist <= PATH_SAMPLE_M {
            return true; // both ends water, shorter than one sample step
        }
        let steps = (dist / PATH_SAMPLE_M).ceil() as usize;
        let dx = (b.longitude - a.longitude) / steps as f64;
        let dy = (b.latitude - a.latitude) / steps as f64;
        for i in 1..steps {
            let probe = GeoPosition {
                latitude: a.latitude + dy * i as f64,
                longitude: a.longitude + dx * i as f64,
            };
            if !self.is_water(&probe) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(lat: f64, lon: f64) -> GeoPosition {
        GeoPosition { latitude: lat, longitude: lon }
    }

    #[test]
    fn asset_loads_and_distinguishes_land_water() {
        let land = Land::from_default_asset().expect("asset parses");
        // Java (inland) is land; the Java Sea north of the coast is
        // water. (Points near 107.0E/-6.0 sit inside Jakarta Bay on NE
        // 50m — checked against the asset, not assumed.)
        assert!(!land.is_water(&pos(-6.2, 107.0)), "inland Java is land");
        assert!(land.is_water(&pos(-5.8, 107.3)), "Java Sea is water");
        // Mid-Atlantic and central Siberia: far from any boundary cases.
        assert!(land.is_water(&pos(0.0, -30.0)));
        assert!(!land.is_water(&pos(62.0, 100.0)), "Siberia is land");
    }

    #[test]
    fn dateline_ring_tests_as_one_strip() {
        // Chukotka (NE 50m ring crosses ±180°): points either side of the
        // antimeridian are both on land.
        let land = Land::from_default_asset().expect("asset parses");
        assert!(!land.is_water(&pos(66.0, 179.5)), "west of dateline");
        assert!(!land.is_water(&pos(66.0, -179.8)), "east of dateline");
        // Bering Sea just off the coast stays water.
        assert!(land.is_water(&pos(64.0, -174.0)));
    }

    #[test]
    fn path_detection_flags_land_crossings() {
        let land = Land::from_default_asset().expect("asset parses");
        // Sea route across the Java Sea: water the whole way.
        assert!(land.path_is_water(&pos(-5.8, 106.5), &pos(-5.6, 107.5)));
        // Straight shot across Java (north coast to south coast) crosses
        // land: must be flagged.
        assert!(!land.path_is_water(&pos(-5.9, 106.8), &pos(-7.0, 106.8)));
        // A path that starts on land is blocked immediately.
        assert!(!land.path_is_water(&pos(-6.9, 107.0), &pos(-6.5, 107.5)));
    }
}
