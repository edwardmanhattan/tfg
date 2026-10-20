//! The map's GEOMETRY, with no MapLibre in it.
//!
//! The Web Mercator projection, the ground resolution it implies, and the
//! place the map's picture goes in the window when the camera has moved on
//! from the camera the picture was rendered at.
//!
//! It is its own module because the GPUI shell needs all of it and the
//! parity proto must not build MapLibre: `proto/gpui-island` compiles this
//! file by path, the way it compiles `view` and `map_gate`, and the
//! projection is arithmetic — it never touches a tile. The map ENGINE (the
//! renderer, the tile cache, the style) stays in `map_render`, which
//! re-exports everything here so that one definition serves both shells.

/// The console's base map geometry, shared by both shells.
///
/// One definition rather than two: the GPUI shell and the egui shell must open
/// on the same place at the same size, or every capture comparison between
/// them is a comparison of two different maps. The egui shell keeps its own
/// names as aliases so its call sites are untouched.
pub const MAP_W: f64 = 800.0;
pub const MAP_H: f64 = 600.0;
pub const MAP_CENTER: (f64, f64) = (-6.108, 106.910);
pub const MAP_ZOOM: f64 = 11.0;
pub const MAP_STYLE: &str = "https://tiles.openfreemap.org/styles/liberty";
/// Pumps for a jump (a recentre); an ordinary gesture spends fewer.
pub const JUMP_PUMP: u32 = 6;
/// Pumps for a pan or zoom step mid-gesture: one render plus one 50ms
/// grace for tiles. Network tiles take longer than any sleep budget when
/// the ground is new, and the cache answers warm ground inside the
/// render — so further sleeps buy almost nothing on fresh ground while
/// each one costs the drag ~60ms of staleness. Jumps and warm-ups keep
/// the full budget: a recentre should land complete, and startup is not
/// latency-sensitive the way a held drag is.
pub const GESTURE_PUMP: u32 = 1;

/// How much one pixel of wheel moves the camera, in zoom levels.
///
/// The egui console's rule, and the reason the map feels the same under
/// the wheel as it did there: a wheel is a gesture and not a button, so the
/// zoom is proportional to the wheel rather than a step per notch.
pub const WHEEL_ZOOM_STEP: f64 = 0.005;

/// How often the map thread is asked for a picture.
///
/// A quarter of a second: long enough that a gesture spends its time
/// moving the camera rather than queueing frames nobody sees, short enough
/// that the tiles have caught up by the time the operator stops. The
/// picture in between is the last one, PLACED by the camera that moved —
/// which is what makes the map track the pointer.
pub const MAP_FRAME_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// How long a window has to stop moving before the map is asked for at its
/// new size.
///
/// The map thread rebuilds its whole scene when the size changes, and a
/// scene is expensive: the style loads and the tile pipeline warms. So the
/// size follows the window only once the window has settled, and a drag of
/// the window's edge costs one rebuild rather than one per frame.
pub const MAP_RESIZE_SETTLE: std::time::Duration = std::time::Duration::from_millis(600);

/// World-size base in px at zoom 0 for the overlay math. Calibrated
/// against the renderer (task #42, `examples/calibrate_projection.rs`):
/// a +0.02° latitude pan at zoom 11 moves content 59.0px on the frame
/// vs 29.3px under a 256 base (ratio 2.014) — the MapLibre world is
/// 512-based, so the overlay must be too, or units drift with zoom.
pub const WORLD_BASE_PX: f64 = 512.0;

/// Earth's equatorial circumference, metres. The numerator of the
/// ground resolution below, kept as a named constant because a second
/// copy of this number is how a scale drifts by a factor of two.
pub const EARTH_EQUATOR_M: f64 = 40_075_016.686;

/// Ground resolution at a latitude and zoom, metres per pixel.
///
/// The inverse of `project_mercator`'s scale: the world is
/// `WORLD_BASE_PX * 2^zoom` pixels wide and one full turn of
/// longitude is `EARTH_EQUATOR_M` metres, with the Mercator
/// stretch's `cos(latitude)` on top. Both constants are the ones the
/// projection above already uses, so the two cannot disagree.
///
/// This is the ONLY conversion from real-world metres to map pixels in
/// the client. Nothing else may scale a hull: an image's aspect ratio
/// is a property of a photograph, not a measurement of a ship.
pub fn meters_per_pixel(latitude: f64, zoom: f64) -> f64 {
    // Mercator diverges at the poles; clamping to the Web Mercator
    // limit keeps a hull at 89.999° finite rather than dividing by a
    // vanishing cosine.
    let lat = latitude.clamp(-MAX_MERCATOR_LAT, MAX_MERCATOR_LAT);
    EARTH_EQUATOR_M * lat.to_radians().cos() / (WORLD_BASE_PX * 2f64.powf(zoom))
}

/// The Web Mercator cut-off latitude. Mercator is defined to ±85.0511°;
/// past it the projection is clamped by every renderer, and this
/// client follows rather than producing infinities.
pub const MAX_MERCATOR_LAT: f64 = 85.051_128_78;

/// Project lat/lon to window pixels for a `w` x `h` viewport centered on
/// `center` at `zoom`.
pub fn project_mercator(
    lat: f64,
    lon: f64,
    center: (f64, f64),
    zoom: f64,
    w: f64,
    h: f64,
) -> (f64, f64) {
    fn world(lat: f64, lon: f64, zoom: f64) -> (f64, f64) {
        let scale = WORLD_BASE_PX * 2f64.powf(zoom);
        let x = (lon + 180.0) / 360.0 * scale;
        let s = (lat.to_radians().tan() + 1.0 / lat.to_radians().cos()).ln();
        let y = (1.0 - s / std::f64::consts::PI) / 2.0 * scale;
        (x, y)
    }
    let (x, y) = world(lat, lon, zoom);
    let (cx, cy) = world(center.0, center.1, zoom);
    (x - cx + w / 2.0, y - cy + h / 2.0)
}

/// The centre a pan drag asks for.
///
/// A drag points at the map pixel under the cursor and asks for THAT GROUND to
/// stay there, so the centre becomes that pixel — which is why the sign lives
/// here and not at the call site: dragging the picture right moves the centre
/// LEFT, and writing that arithmetic twice is how the two directions come to
/// disagree.
pub fn pan_center(
    center: (f64, f64),
    zoom: f64,
    dx: f64,
    dy: f64,
    w: f64,
    h: f64,
) -> (f64, f64) {
    unproject_mercator(w / 2.0 - dx, h / 2.0 - dy, center, zoom, w, h)
}

/// Keep the point under the cursor fixed across a zoom step (task #43):
/// the new center puts the cursor's geographic point back under it.
/// Exact inverse of the forward path by construction (see test).
#[allow(clippy::too_many_arguments)]
pub fn anchor_center(
    px: f64,
    py: f64,
    center: (f64, f64),
    zoom: f64,
    new_zoom: f64,
    w: f64,
    h: f64,
) -> (f64, f64) {
    use std::f64::consts::PI;
    let (gla, glo) = unproject_mercator(px, py, center, zoom, w, h);
    let scale = WORLD_BASE_PX * 2f64.powf(new_zoom);
    let gx = (glo + 180.0) / 360.0 * scale;
    let s = (gla.to_radians().tan() + 1.0 / gla.to_radians().cos()).ln();
    let gy = (1.0 - s / PI) / 2.0 * scale;
    let cx = gx - (px - w / 2.0);
    let cy = gy - (py - h / 2.0);
    let lon = cx / scale * 360.0 - 180.0;
    let s2 = (1.0 - 2.0 * cy / scale) * PI;
    let lat = s2.sinh().atan().to_degrees();
    (lat, lon)
}

/// Inverse of [`project_mercator`]: window pixels back to lat/lon.
/// Used for click-to-order waypoints; exact round-trip of the forward path.
#[allow(clippy::too_many_arguments)]
pub fn unproject_mercator(
    px: f64,
    py: f64,
    center: (f64, f64),
    zoom: f64,
    w: f64,
    h: f64,
) -> (f64, f64) {
    use std::f64::consts::PI;
    fn world(lat: f64, lon: f64, zoom: f64) -> (f64, f64) {
        let scale = WORLD_BASE_PX * 2f64.powf(zoom);
        let x = (lon + 180.0) / 360.0 * scale;
        let s = (lat.to_radians().tan() + 1.0 / lat.to_radians().cos()).ln();
        let y = (1.0 - s / PI) / 2.0 * scale;
        (x, y)
    }
    let scale = WORLD_BASE_PX * 2f64.powf(zoom);
    let (cx, cy) = world(center.0, center.1, zoom);
    let x = px - w / 2.0 + cx;
    let y = py - h / 2.0 + cy;
    let lon = x / scale * 360.0 - 180.0;
    let s = (1.0 - 2.0 * y / scale) * PI;
    let lat = s.sinh().atan().to_degrees();
    (lat, lon)
}

/// A rectangle, in window points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    /// The right edge.
    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    /// The bottom edge.
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }
}

/// Where the map frame sits in the window once the camera has moved on from
/// the camera the frame was RENDERED at.
///
/// The two are different numbers, and the difference is the whole point: a
/// gesture moves the camera at once a frame, while the map thread needs a
/// fifth of a second (and a fresh set of tiles) to catch up. So the picture
/// on screen is the LAST frame, placed where the CURRENT camera says it
/// belongs — the same trick the egui console used with a texture and a UV
/// rectangle, expressed here as a rectangle and a scale because GPUI paints
/// an image into a rectangle and has no UV rectangle to give it.
///
/// Everything here is one uniform scale and one translation: the frame's
/// pixels and the live camera's pixels are the same ground at two zooms, so
/// no skew can creep in, and the marker layer (which is drawn in the live
/// camera's pixels) and the picture (drawn in the frame's) can never be two
/// different transforms of the same water.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FramePlacement {
    /// The frame's rectangle: where the picture goes.
    pub frame: Rect,
    /// The rectangle the frame fills when the camera matches it — the area
    /// the map is meant to cover.
    pub cover: Rect,
    /// The frame's own pixels to window points.
    pub scale: f64,
    /// The band on the left the frame does not reach, if any. The frame's
    /// left column smears into it.
    pub left: Option<Rect>,
    /// The band on the right, if any. The frame's right column smears into
    /// it.
    pub right: Option<Rect>,
    /// The band along the top, if any. The frame's top row smears into it.
    pub top: Option<Rect>,
    /// The band along the bottom, if any.
    pub bottom: Option<Rect>,
}

/// The world position of a ground point at a zoom, in the 512-based world
/// pixels the projection above uses.
///
/// The same arithmetic [`project_mercator`] does, named once so the
/// placement below cannot drift from it by a factor of two.
fn world_px(lat: f64, lon: f64, zoom: f64) -> (f64, f64) {
    let scale = WORLD_BASE_PX * 2f64.powf(zoom);
    let x = (lon + 180.0) / 360.0 * scale;
    let s = (lat.to_radians().tan() + 1.0 / lat.to_radians().cos()).ln();
    let y = (1.0 - s / std::f64::consts::PI) / 2.0 * scale;
    (x, y)
}

/// Where the map frame sits in the window: a UNIFORM scale that covers the
/// window, plus the top-left origin of the scaled frame.
///
/// The frame and the window rarely have the same shape, so one of the two is
/// cropped. The origin is negative on the cropped axis (the frame is
/// CENTRED, not anchored at a corner), and it is what both the painted
/// picture and the marker layer must use: sizing them from the window's
/// width and height independently is how two different transforms of the
/// same ground happen, and they drift apart the moment the camera moves.
///
/// Cover and not contain, because this is a viewport and a letterboxed map
/// would show the black it is meant to replace.
pub fn cover_placement(frame_px: (f64, f64), window: (f64, f64)) -> (f64, f64, f64) {
    if frame_px.0 <= 0.0 || frame_px.1 <= 0.0 || window.0 <= 0.0 || window.1 <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let scale = (window.0 / frame_px.0).max(window.1 / frame_px.1);
    let ox = (window.0 - frame_px.0 * scale) / 2.0;
    let oy = (window.1 - frame_px.1 * scale) / 2.0;
    (scale, ox, oy)
}

/// Where the map frame goes in the window, and the bands it does not reach.
///
/// `live` is the camera the operator is looking at; `frame` is the camera
/// the picture was rendered at. They are the same camera when the map thread
/// has caught up, and the frame's rectangle is then exactly the cover
/// rectangle with no bands — which is the honest picture of a settled map.
///
/// `frame_px` is the frame's own pixel size and `window` the window's size in
/// points. The two are cover-placed into each other: one uniform scale, the
/// frame centred. Cover and not contain, because this is a viewport and a
/// letterboxed map would show the black it is meant to replace.
pub fn place_frame(
    live: (f64, f64),
    live_zoom: f64,
    frame: (f64, f64),
    frame_zoom: f64,
    frame_px: (f64, f64),
    window: (f64, f64),
) -> FramePlacement {
    let (cover_scale, ox, oy) = cover_placement(frame_px, window);
    let cover = Rect {
        x: ox,
        y: oy,
        w: frame_px.0 * cover_scale,
        h: frame_px.1 * cover_scale,
    };

    // The frame's pixels and the live camera's pixels are the same ground
    // seen at two zooms, so one is the other scaled by the zoom difference
    // and moved: `k` is the frame's zoom over the live one, and `b` is the
    // translation in frame pixels. Both come out of the world positions
    // directly, so the relation is exact rather than fitted.
    let k = 2f64.powf(frame_zoom - live_zoom);
    let (lx, ly) = world_px(live.0, live.1, live_zoom);
    let (fx, fy) = world_px(frame.0, frame.1, frame_zoom);
    let bx = k * lx - k * (frame_px.0 / 2.0) - fx + frame_px.0 / 2.0;
    let by = k * ly - k * (frame_px.1 / 2.0) - fy + frame_px.1 / 2.0;

    // The live pixel of a frame pixel is `(frame - b) / k`, and the live
    // pixel lands at the cover placement. So the frame's rectangle in the
    // window is the cover rectangle, scaled by `1 / k` and moved by
    // `-b / k`.
    let scale = cover_scale / k;
    let frame_rect = Rect {
        x: ox - (bx / k) * cover_scale,
        y: oy - (by / k) * cover_scale,
        w: frame_px.0 * scale,
        h: frame_px.1 * scale,
    };

    // The bands the frame does not reach: where the live camera has moved
    // off the frame's own coverage and the picture has no ground to show.
    // Each band is the frame's own edge, smeared outward — the map's
    // colours instead of a hole, which is what a texture sampled with
    // clamped edges gives and what the egui console's stale texture gave.
    //
    // The band spans the COVER rectangle on its other axis, not the frame's:
    // the smear is one pixel wide, and one pixel stretched over a slightly
    // different height is a difference nobody can see. The corner where two
    // bands meet is painted twice, and the second one wins — again a
    // one-pixel difference nobody can see.
    let left = (cover.x < frame_rect.x).then(|| Rect {
        x: cover.x,
        y: cover.y,
        w: frame_rect.x - cover.x,
        h: cover.h,
    });
    let right = (frame_rect.right() < cover.right()).then(|| Rect {
        x: frame_rect.right(),
        y: cover.y,
        w: cover.right() - frame_rect.right(),
        h: cover.h,
    });
    let top = (cover.y < frame_rect.y).then(|| Rect {
        x: cover.x,
        y: cover.y,
        w: cover.w,
        h: frame_rect.y - cover.y,
    });
    let bottom = (frame_rect.bottom() < cover.bottom()).then(|| Rect {
        x: cover.x,
        y: frame_rect.bottom(),
        w: cover.w,
        h: cover.bottom() - frame_rect.bottom(),
    });

    FramePlacement {
        frame: frame_rect,
        cover,
        scale,
        left,
        right,
        top,
        bottom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

/// The picture and the markers must be ONE transform of the ground.
///
/// The picture is the last frame, placed by the camera the operator is
/// holding; the markers are drawn at that camera directly. If the two
/// transforms disagreed by a pixel, a hull would sit beside its own
/// water — and the disagreement would grow with every zoom, because the
/// picture is the one being scaled.
#[test]
fn the_picture_and_the_markers_are_one_transform() {
    let frame_px = (800.0_f64, 600.0_f64);
    let window = (760.0_f64, 620.0_f64);
    // The camera the operator is holding, and the older one the picture
    // was rendered at: a zoom the map thread has not caught up with.
    let live = (-6.108, 106.910);
    let live_zoom = 12.0;
    let frame = (-6.108, 106.910);
    let frame_zoom = 11.0;
    let placement = place_frame(live, live_zoom, frame, frame_zoom, frame_px, window);

    // A ground point, and where each of the two transforms puts it.
    let ground = (-6.1, 106.9);
    let (tx, ty) = project_mercator(
        ground.0,
        ground.1,
        frame,
        frame_zoom,
        frame_px.0,
        frame_px.1,
    );
    let picture = (
        placement.frame.x + tx * placement.scale,
        placement.frame.y + ty * placement.scale,
    );
    let (mx, my) = project_mercator(
        ground.0,
        ground.1,
        live,
        live_zoom,
        frame_px.0,
        frame_px.1,
    );
    let (scale, ox, oy) = cover_placement(frame_px, window);
    let marker = (ox + mx * scale, oy + my * scale);

    assert!(
        (picture.0 - marker.0).abs() < 1e-6 && (picture.1 - marker.1).abs() < 1e-6,
        "the picture is at {picture:?} and the marker at {marker:?}: two transforms of one ground"
    );
}

/// A settled map — the camera and the frame agree — covers the window
/// exactly and leaves no band anywhere. This is the case that must not
/// move: a band here would be a hole in a map that has nothing to
/// apologise for.
#[test]
fn a_settled_map_covers_the_window_with_no_bands() {
    let centre = (-6.108, 106.910);
    let (w, h) = (800.0_f64, 600.0_f64);
    let placement = place_frame(centre, 11.0, centre, 11.0, (w, h), (w, h));

    assert!(
        (placement.frame.x - placement.cover.x).abs() < 1e-9
            && (placement.frame.y - placement.cover.y).abs() < 1e-9
            && (placement.frame.w - placement.cover.w).abs() < 1e-9
            && (placement.frame.h - placement.cover.h).abs() < 1e-9,
        "the frame sits on the cover: {:?} vs {:?}",
        placement.frame,
        placement.cover
    );
    assert!(
        placement.left.is_none()
            && placement.right.is_none()
            && placement.top.is_none()
            && placement.bottom.is_none(),
        "a settled map has no band: {:?}",
        placement
    );
}

/// The ground under the cursor does not move when the camera zooms.
///
/// This is the property the whole placement exists for: the picture on
/// screen is the LAST frame, and it has to be put where the CURRENT
/// camera says the ground is, or a hull slides off its own water while
/// the operator zooms.
#[test]
fn the_ground_under_the_cursor_stays_put_across_a_zoom() {
    let centre = (-6.108, 106.910);
    let (w, h) = (800.0_f64, 600.0_f64);
    let (px, py) = (200.0_f64, 420.0_f64);

    // The ground under the cursor, at the camera before the zoom.
    let ground = unproject_mercator(px, py, centre, 11.0, w, h);
    // The camera after a zoom that keeps that ground under the cursor.
    let zoomed = anchor_center(px, py, centre, 11.0, 12.3, w, h);

    // The picture is still the OLD camera's frame.
    let placement = place_frame(zoomed, 12.3, centre, 11.0, (w, h), (w, h));
    // Where the frame's own pixel of that ground lands in the window.
    let (fx, fy) = project_mercator(ground.0, ground.1, centre, 11.0, w, h);
    let wx = placement.frame.x + fx * placement.scale;
    let wy = placement.frame.y + fy * placement.scale;

    assert!(
        (wx - px).abs() < 0.01,
        "the ground under the cursor slid to {wx}, wanted {px}"
    );
    assert!(
        (wy - py).abs() < 0.01,
        "the ground under the cursor slid to {wy}, wanted {py}"
    );
}

/// A pan slides the picture WITH the drag, and the band it leaves is on
/// the side the camera went — the side the frame's coverage no longer
/// reaches.
#[test]
fn a_pan_slides_the_picture_with_the_drag_and_bands_the_side_the_camera_went() {
    let centre = (-6.108, 106.910);
    let (w, h) = (800.0_f64, 600.0_f64);
    // The drag went right, so the camera went left and the picture
    // follows the hand: a pan is 1:1 with the pointer.
    let moved = pan_center(centre, 11.0, 40.0, 0.0, w, h);
    let placement = place_frame(moved, 11.0, centre, 11.0, (w, h), (w, h));

    assert!(
        (placement.frame.x - 40.0).abs() < 0.01,
        "the picture slid to {}, wanted 40",
        placement.frame.x
    );
    let left = placement.left.expect("the camera went left, so the left is short");
    assert!(
        (left.w - 40.0).abs() < 0.01,
        "the band is as wide as the pan: {}",
        left.w
    );
    assert!(
        placement.right.is_none(),
        "and the side the camera reached is still covered"
    );
}

/// A zoom IN magnifies the stale picture about the camera, so the frame's
/// rectangle grows by the zoom difference — the picture is a bigger view
/// of the same ground, not a different ground.
#[test]
fn a_zoom_in_magnifies_the_stale_picture() {
    let centre = (-6.108, 106.910);
    let (w, h) = (800.0_f64, 600.0_f64);
    let placement = place_frame(centre, 12.0, centre, 11.0, (w, h), (w, h));

    let ratio = 2f64.powf(12.0 - 11.0);
    assert!(
        (placement.frame.w - w * ratio).abs() < 0.01,
        "the picture grew to {}, wanted {}",
        placement.frame.w,
        w * ratio
    );
}
}
