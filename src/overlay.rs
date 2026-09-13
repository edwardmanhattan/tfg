//! Overlay hit-testing (screen-space, pixels).
//!
//! Companion to the [`project_mercator`](crate::map_render::project_mercator)
//! seam (ADR-0001): markers are projected to window pixels by the shell,
//! and pointer hits resolve back to ship ids here. Pure geometry, so the
//! command-center inspector is unit-testable without a window.

/// Nearest marker id within `radius_px` of a pointer at `(px, py)`.
///
/// `markers` are `(ship_id, x, y)` in the same pixel space as the pointer
/// (callers filter hidden markers first). Ties resolve to the first marker.
pub fn hit_test(markers: &[(String, f64, f64)], px: f64, py: f64, radius_px: f64) -> Option<String> {
    let mut best: Option<(f64, &str)> = None;
    for (id, x, y) in markers {
        let d = ((x - px).powi(2) + (y - py).powi(2)).sqrt();
        if d <= radius_px && best.map(|(bd, _)| d < bd).unwrap_or(true) {
            best = Some((d, id));
        }
    }
    best.map(|(_, id)| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn markers() -> Vec<(String, f64, f64)> {
        vec![
            ("a".to_string(), 100.0, 100.0),
            ("b".to_string(), 200.0, 100.0),
        ]
    }

    #[test]
    fn hit_inside_radius() {
        assert_eq!(hit_test(&markers(), 105.0, 100.0, 12.0).as_deref(), Some("a"));
    }

    #[test]
    fn miss_outside_radius() {
        assert_eq!(hit_test(&markers(), 150.0, 100.0, 12.0), None);
    }

    #[test]
    fn nearest_wins_between_two() {
        assert_eq!(hit_test(&markers(), 160.0, 100.0, 100.0).as_deref(), Some("b"));
    }

    #[test]
    fn empty_markers_never_hit() {
        assert_eq!(hit_test(&[], 100.0, 100.0, 12.0), None);
    }
}
