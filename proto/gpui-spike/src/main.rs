//! GPUI spike. One question: does GPUI render a real frame on this machine?
//!
//! It exits 0 only if the compositor maps the window and the render callback
//! puts pixels on screen. That is the stop-or-go gate for the migration.
//!
//! ADR-0018 measured a negative on this question against the project's Xvfb
//! on `:99`. That measurement was against a virtual display with no DRI3.
//! This machine now has a Hyprland session on `wayland-1` and real Vulkan
//! hardware, so the question has to be asked again.

use gpui::{
    App, Application, Bounds, Context, FontWeight, IntoElement, ParentElement, Render, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};

struct Spike {
    frames: u64,
}

impl Render for Spike {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // `request_animation_frame` re-notifies this view on the next frame,
        // so a non-zero frame count is proof the compositor is presenting.
        window.request_animation_frame();
        self.frames += 1;

        div()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x101820))
            .size_full()
            .p_6()
            .child(div().h(px(60.0)).w_full().bg(rgb(0x22d3ee)))
            .child(div().h(px(60.0)).w_full().bg(rgb(0xf59e0b)))
            .child(div().h(px(60.0)).w_full().bg(rgb(0x4ade80)))
            .child(
                div()
                    .mt_4()
                    .text_color(rgb(0xffffff))
                    .child(format!("gpui spike - frame {}", self.frames)),
            )
            // Real font weights. This is the reason the migration was asked
            // for, and egui cannot do it without one family per weight.
            .child(
                div()
                    .flex()
                    .gap_6()
                    .child(
                        div()
                            .font_weight(FontWeight::LIGHT)
                            .text_color(rgb(0xffffff))
                            .child("LIGHT 300"),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(rgb(0xffffff))
                            .child("NORMAL 400"),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(0xffffff))
                            .child("MEDIUM 500"),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(0xffffff))
                            .child("BOLD 700"),
                    ),
            )
            // Translucency. The other reason, and the one egui cannot do at all.
            .child(
                div()
                    .bg(gpui::hsla(0.55, 0.9, 0.6, 0.45))
                    .p_4()
                    .text_color(rgb(0xffffff))
                    .child("translucent - alpha 0.45"),
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(520.0), px(460.0)), cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        match cx.open_window(opts, |_, cx| cx.new(|_| Spike { frames: 0 })) {
            Ok(_) => eprintln!("spike: window opened"),
            Err(e) => {
                eprintln!("spike: window FAILED to open: {e:?}");
                std::process::exit(2);
            }
        }
        cx.activate(true);
    });
}
