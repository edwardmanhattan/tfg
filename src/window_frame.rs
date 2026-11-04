//! How a console window opens, and the one verb that changes it.
//!
//! WHY THIS EXISTS. Both windows opened at a fixed size — 760x620 for the
//! console, 1280x800 for the command center — whatever display they landed on,
//! and on WSLg there is no frame to drag: WSLg's Weston draws no server-side
//! decorations, GPUI's Wayland backend draws no client-side ones, and
//! `start_window_resize` exists in GPUI but nothing in this app calls it,
//! because there is no handle on screen to call it from. So a window that
//! opened small stayed small, on a 1920x1200 display, with no way to say
//! otherwise short of editing the source.
//!
//! THE TWO ANSWERS, both here. The window opens at a SHARE OF THE DISPLAY,
//! because the display is the only thing that knows how much room there is
//! (and the layout's designed size is the floor, not the target); and F11
//! toggles fullscreen, which is the one resize a compositor with no frame can
//! still honour. `TFG_WINDOW_SIZE=<width>x<height>` pins an opening size
//! outright for an operator who knows the wall the console hangs on.
//!
//! WHY THE ACTION AND NOT A KEY HANDLER ON AN ELEMENT. The console's frame is
//! built by whichever surface is up (signed out, flow window, sheet, console),
//! so the four of them have no element in common to hang a handler on. A
//! keymap binding dispatches along the focus path to the dispatch tree's root,
//! so the listener goes on the root that [`with_window_verbs`] wraps around all
//! four — and it works with nothing focused, which is the normal state of a
//! console that has just opened.

use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, KeyBinding, ParentElement, Pixels, Size,
    Styled, Window, div, px, size,
};

gpui::actions!(tfg_window, [ToggleFullscreen]);

/// The share of the display a window opens at, before the designed size has
/// its say. Four fifths and a bit leaves the desktop's own edges visible, so
/// an operator can still see the window is a window.
const OPENING_SHARE: f32 = 0.85;

/// The smallest window worth opening at, whatever the display says.
const FLOOR: (f32, f32) = (640.0, 480.0);

/// Bind the window's own verbs. Called once, before any window is opened.
pub fn bind_window_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("f11", ToggleFullscreen, None)]);
}

/// Wrap a rendered surface so a window verb dispatched from anywhere in it
/// reaches the window.
///
/// The wrapper is a plain full-size box: it exists to be the dispatch tree's
/// root, not to lay anything out.
pub fn with_window_verbs(surface: AnyElement) -> AnyElement {
    div()
        .id("window-verbs")
        .size_full()
        .on_action(|_: &ToggleFullscreen, window: &mut Window, _cx: &mut App| {
            window.toggle_fullscreen()
        })
        .child(surface)
        .into_any_element()
}

/// The size a window opens at and the floor it may not be dragged below.
pub struct OpeningSize {
    pub size: Size<Pixels>,
    pub min: Size<Pixels>,
}

impl OpeningSize {
    /// `designed` is the size the layout was designed at. The window opens at
    /// the larger of that and [`OPENING_SHARE`] of the display, is capped at
    /// the display itself (a window larger than its screen cannot be dragged
    /// back into view), and may not be dragged below `designed`.
    pub fn for_display(cx: &App, designed: Size<Pixels>) -> Self {
        let designed_width = f32::from(designed.width);
        let designed_height = f32::from(designed.height);
        // A pinned size wins — including a pinned size BELOW the designed one,
        // because an operator who names a number is reporting a display this
        // code cannot see. The floor follows it down, so the compositor is not
        // handed a minimum its window already violates.
        if let Some(pinned) = pinned_size() {
            let width = f32::from(pinned.width);
            let height = f32::from(pinned.height);
            return Self {
                size: pinned,
                min: size(
                    px(width.min(designed_width)),
                    px(height.min(designed_height)),
                ),
            };
        }
        let Some(display_size) = display_size(cx) else {
            return Self {
                size: designed,
                min: designed,
            };
        };
        let display_width = f32::from(display_size.width);
        let display_height = f32::from(display_size.height);
        let target = size(
            px((display_width * OPENING_SHARE)
                .max(designed_width)
                .max(FLOOR.0)
                .min(display_width)),
            px((display_height * OPENING_SHARE)
                .max(designed_height)
                .max(FLOOR.1)
                .min(display_height)),
        );
        Self {
            size: target,
            min: size(
                px(designed_width.min(f32::from(target.width))),
                px(designed_height.min(f32::from(target.height))),
            ),
        }
    }
}

/// The size of the display a window should open on.
///
/// NOT `App::primary_display()`. GPUI 0.2.2's Wayland backend answers `None`
/// from it unconditionally (`vendor/gpui/src/platform/linux/wayland/client.rs`,
/// `fn primary_display`) while populating `displays()` from `wl_output` events,
/// so a Wayland desktop — WSLg included — reported no display at all, and every
/// window fell back to its designed size. The list is asked for instead, and the
/// LARGEST output wins: a command centre is hung on the biggest screen in the
/// room, and opening smaller because another output happens to sort first would
/// be the same bug with a different number.
fn display_size(cx: &App) -> Option<Size<Pixels>> {
    if let Some(display) = cx.primary_display() {
        return Some(display.bounds().size);
    }
    let area = |size: Size<Pixels>| f32::from(size.width) * f32::from(size.height);
    cx.displays()
        .into_iter()
        .map(|display| display.bounds().size)
        .max_by(|a, b| area(*a).total_cmp(&area(*b)))
}

/// `TFG_WINDOW_SIZE=1920x1080` — an operator pinning the size the console
/// opens at, instead of taking the share of the display.
fn pinned_size() -> Option<Size<Pixels>> {
    let raw = std::env::var("TFG_WINDOW_SIZE").ok()?;
    match parse_size(&raw) {
        Some(size) => Some(size),
        None => {
            // Said out loud, once, rather than silently ignored: a typo in a
            // wall display's size would otherwise read as "the pin does
            // nothing".
            eprintln!("TFG_WINDOW_SIZE={raw:?} is not WIDTHxHEIGHT; opening at the display share");
            None
        }
    }
}

/// `1600x1000`, `1600X1000`, or either with spaces. Small numbers are refused:
/// a 12x9 window is a typo, not a size.
fn parse_size(raw: &str) -> Option<Size<Pixels>> {
    let (width, height) = raw.trim().split_once(['x', 'X'])?;
    let width: f32 = width.trim().parse().ok()?;
    let height: f32 = height.trim().parse().ok()?;
    if !width.is_finite() || !height.is_finite() {
        return None;
    }
    if width < FLOOR.0 || height < FLOOR.1 {
        return None;
    }
    Some(size(px(width), px(height)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pinned_size_is_read_in_either_case() {
        let parsed = parse_size("1600x1000").expect("lower case x");
        assert_eq!(
            (f32::from(parsed.width), f32::from(parsed.height)),
            (1600.0, 1000.0)
        );
        let parsed = parse_size(" 1920 X 1200 ").expect("upper case X and spaces");
        assert_eq!(
            (f32::from(parsed.width), f32::from(parsed.height)),
            (1920.0, 1200.0)
        );
    }

    #[test]
    fn a_pinned_size_that_is_not_a_size_is_refused() {
        // Each of these is a typo somebody would make on a wall display.
        for raw in ["", "1920", "x1080", "1920x", "widexhigh", "0x0", "12x9"] {
            assert!(parse_size(raw).is_none(), "{raw:?} was accepted");
        }
    }
}
