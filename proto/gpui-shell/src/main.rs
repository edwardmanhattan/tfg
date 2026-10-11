//! U2 shell: the first GPUI window of the migration, now fed by the real
//! backend thread.
//!
//! Slice 3 wires `tfg::shell_threads::spawn_poll_thread` into the scaffold:
//! the same boot wire (`TFG_BACKEND_URL` / `TFG_SCENARIO`, default empty
//! replay), the same 2 s cadence, the same journal-degrades-to-disabled
//! rule. A drain task polls the fix channel on a 250 ms timer and
//! `notify()`s only when fixes arrive — the first retained-mode proof of
//! the U2 notify policy (no repaint timer; arrival-driven updates).
//!
//! Still out of scope: theme/fonts (`apply_ops_theme`, glyph fallbacks),
//! the map thread (egui-tainted `ColorImage` handoff, U6), and the timed
//! `notify_after` sites (step-gate, roster door, halo flash).
//!
//! Status: written, capture-pending. A window is only "working" after the
//! real binary is driven and captured (see the handoff's U2 log).

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::Duration;

use gpui::{
    App, AppContext, Application, AsyncApp, Bounds, Context, IntoElement, ParentElement, Render,
    Styled, TitlebarOptions, Window, WindowBounds, WindowOptions, div, px, size,
};
use tfg::geo::track::Fix;
use tfg::shell_threads::{PollThreadConfig, WireKind, spawn_poll_thread};

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

struct Root {
    fixes: usize,
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgba(CONSOLE_NIGHT))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .text_size(px(TITLE_SIZE))
            .text_color(rgba(MAP_INK))
            .child("tfg // gpui shell")
            .child(format!("{} fixes", self.fixes))
    }
}

/// Drain the poll thread's fix channel without ever blocking the UI thread:
/// a 250 ms timer gates `try_recv`, and `notify()` fires only on arrival.
/// The receiver is owned by the task; `try_recv` borrows it briefly and
/// never holds a borrow across `.await`. `Task::detach` keeps the loop
/// alive past this function.
fn main() {
    Application::new().run(|cx: &mut App| {
        // Channels the poll thread owns one end of; the shell keeps the
        // UI-side ends it needs and drops the rest at quit.
        let shutdown = Arc::new(AtomicBool::new(false));
        let armed = Arc::new(AtomicBool::new(false));
        let (poll_tx, poll_rx) = mpsc::channel();
        let (sim_cmd_tx, sim_cmd_rx) = mpsc::channel();
        let (sim_evt_tx, sim_evt_rx) = mpsc::channel();
        let (wire_ctl_tx, wire_ctl_rx) = mpsc::channel::<WireKind>();
        let (_wire_wake_tx, wire_wake_rx) = mpsc::channel::<()>();
        let _ = (sim_cmd_tx, sim_evt_rx, wire_ctl_tx);

        let _poll_handle = spawn_poll_thread(PollThreadConfig {
            wire_ctl_rx,
            wire_wake_rx,
            sim_cmd_rx,
            sim_evt_tx,
            poll_tx,
            shutdown: shutdown.clone(),
            armed,
            initial_log: std::env::temp_dir().join("gpui-shell-journal.log"),
        });

        // Mirrors `ShipApp::on_exit`: signal shutdown so the poll loop sees
        // it within one 100 ms sleep. Handlers get ~100ms, so no blocking
        // join — the thread exits itself on the flag and the channels drop.
        // `detach()` keeps the observer alive past this closure.
        let quit_shutdown = shutdown.clone();
        cx.on_app_quit(move |_cx: &mut App| {
            let quit_shutdown = quit_shutdown.clone();
            async move {
                quit_shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        })
        .detach();

        let entity = cx.new(|_| Root { fixes: 0 });

        // Arrival-driven drain: 250 ms timer, `try_recv`, notify on fixes.
        let drain_entity = entity.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let rx: mpsc::Receiver<Vec<Fix>> = poll_rx;
            loop {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let mut n = 0;
                for fixes in rx.try_iter() {
                    n += fixes.len();
                }
                if n > 0 {
                    let _ = cx.update(|app| {
                        drain_entity.update(app, |root: &mut Root, cx| {
                            root.fixes += n;
                            cx.notify();
                        })
                    });
                }
            }
        })
        .detach();

        // Same footprint the egui shell opens: 1040x640. `app_id` is set so
        // captures can match the window by class instead of PID.
        let bounds = Bounds::centered(None, size(px(1040.0), px(640.0)), cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("tfg command center (gpui)".into()),
                appears_transparent: false,
                traffic_light_position: None,
            }),
            app_id: Some("gpui-shell".to_string()),
            ..Default::default()
        };
        cx.open_window(opts, |_window, _cx| entity.clone())
            .unwrap();
        cx.activate(true);
    });
}
