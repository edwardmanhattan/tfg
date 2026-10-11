//! U2 shell: the first GPUI window of the migration.
//!
//! One `Entity`, one `Render`, one label. This exists to prove the shell
//! seam, not to operate the console: threads, theme, fonts and the repaint
//! policy arrive in later slices. The shipped egui binary (`src/main.rs`)
//! is untouched; U8 is where the shells swap.
//!
//! Seam list (grounded 2026-10-12, `src/main.rs` refs are the egui side):
//!
//! - Threads: poll spawn `:21270`, map spawn `:21342`. Both live in the
//!   binary, not the lib, so a later slice extracts them into additive
//!   `tfg::shell_threads` spawn fns (verbatim moves, no behaviour change)
//!   that both binaries call. This scaffold opens no channels.
//! - Viewport: `NativeOptions.viewport` inner `[1040.0, 640.0]` (`:21385`)
//!   becomes the centred bounds below. No title builder, no position, no
//!   `on_close` on the egui side; the title travels in `TitlebarOptions`.
//! - Quit: `ShipApp::on_exit` (`:18968`: shutdown flag, drop `map_req_tx`
//!   so the map thread exits, join handles) becomes `on_app_quit`. Two
//!   facts constrain the port: handlers get ~100ms before the process
//!   exits, so joins must be bounded; and the returned `Subscription`
//!   unsubscribes on drop, so it is `detach()`ed, not `let _`.
//! - Repaint: `WATCHDOG_REPAINT` 2s (`:464`), `REST_POLL_REPAINT` 50ms
//!   (`:467`), central policy `:19063`. Becomes `cx.notify()` on dirty /
//!   channel pump / gesture plus timer tasks; the 2s watchdog is a global
//!   timer until the shell is fully event-driven.
//! - Theme/fonts: `apply_ops_theme` (`:21093`), `add_symbol_fallbacks`
//!   (`:1930`). The label below reads `view` tokens directly, which is
//!   the whole of the theme seam at this stage.
//!
//! Status: scaffold written, NOT verified. Per the repo's UI rule a window
//! is only "working" after the real binary is driven and captured on the
//! Wayland compositor (`grim` on `wayland-1`, crop physical = logical *
//! 1.25). That capture is the next step, not this commit.

use gpui::{
    App, AppContext, Application, Bounds, Context, IntoElement, ParentElement, Render, Styled,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, px, size,
};

#[path = "../../../src/view/mod.rs"]
mod view;

use view::*;

/// One `view::Rgba` as a GPUI colour. Same boundary rule as
/// `proto/gpui-island`: `gpui::rgba` is raw 0..1 with no gamma, the exact
/// convention of `Rgba::as_f32`, so values pass through untouched. Never
/// via `Hsla`.
fn rgba(c: Rgba) -> gpui::Rgba {
    let [r, g, b, _] = c.as_f32();
    gpui::Rgba {
        r,
        g,
        b,
        a: c.alpha_f32(),
    }
}

struct Root;

impl Render for Root {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgba(CONSOLE_NIGHT))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(TITLE_SIZE))
            .text_color(rgba(MAP_INK))
            .child("tfg // gpui shell")
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        // Mirrors `ShipApp::on_exit`: signal shutdown, drop the map request
        // sender, join bounded. No threads yet, so the body is a marker for
        // the thread-wiring slice. `detach()` keeps the observer alive past
        // this closure; a bare `let _` would unsubscribe on return.
        cx.on_app_quit(|_cx: &mut App| async {}).detach();

        // Same footprint the egui shell opens: 1040x640.
        let bounds = Bounds::centered(None, size(px(1040.0), px(640.0)), cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("tfg command center (gpui)".into()),
                appears_transparent: false,
                traffic_light_position: None,
            }),
            ..Default::default()
        };
        cx.open_window(opts, |_window, cx| cx.new(|_| Root))
            .unwrap();
        cx.activate(true);
    });
}
