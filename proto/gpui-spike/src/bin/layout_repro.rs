//! Layout repro. One question: in GPUI 0.2.2, does an absolutely positioned
//! STRETCHED container paint its children?
//!
//! The console's `top_zone` is built `.absolute().top(0).left(0).right(0).h(n)`
//! with `.px()` padding and children, and its children render. The map chrome
//! was built the same way and its children did not, across eight attempts.
//! Rather than keep bisecting inside a 21k-line console, this crate puts the
//! variants side by side in one window so a single capture settles it.
//!
//! Each band is labelled with the property it isolates. A band whose LABEL
//! and whose INNER BOX both appear is a pass; a band showing only its own
//! background is a fail.

use gpui::{
    AnyElement, App, MouseMoveEvent, Application, Bounds, Context, Div, IntoElement, ParentElement, Render, SharedString,
    Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};

const NIGHT: u32 = 0x0f172a;
const CYAN: u32 = 0x22d3ee;
const AMBER: u32 = 0xf59e0b;
const INK: u32 = 0xe2e8f0;
const PANEL: u32 = 0x1e293b;

/// A band: a stretched strip with a label and a probe box inside it. If the
/// probe box renders, the strip laid its children out.
fn band(label: &'static str, build: impl FnOnce(Div) -> Div) -> Div {
    let strip = div()
        .absolute()
        .left(px(0.0))
        .right(px(0.0))
        .h(px(64.0))
        .bg(rgb(NIGHT))
        .flex()
        .flex_row()
        .items_center();
    build(strip)
        // The label, appended last so it is present whatever the strip does.
        .child(
            div()
                .pl(px(12.0))
                .text_color(rgb(INK))
                .text_size(px(13.0))
                .child(SharedString::from(label)),
        )
        // The probe: a solid box that is unmistakable in a capture.
        .child(
            div()
                .w(px(40.0))
                .h(px(28.0))
                .bg(rgb(AMBER))
                .flex_shrink_0(),
        )
}

/// The console's exact path: a function returning `Vec<AnyElement>`, added to
/// the root in a loop rather than chained with `.child()`.
fn chrome_vec(top: f32, py: f32, h: f32) -> Vec<gpui::AnyElement> {
    let strip = div()
        .absolute()
        .left(px(0.0))
        .right(px(0.0))
        .top(px(top))
        .h(px(h))
        .bg(rgb(NIGHT))
        .flex()
        .flex_row()
        .items_center()
        .py(px(py))
        .px(px(12.0))
        .child(
            div()
                .text_color(rgb(INK))
                .text_size(px(13.0))
                .child("H: Vec<AnyElement> + loop"),
        )
        .child(div().w(px(40.0)).h(px(28.0)).bg(rgb(AMBER)).flex_shrink_0());
    vec![strip.into_any_element()]
}

struct Repro {
    frames: u64,
}

impl Render for Repro {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        window.request_animation_frame();
        self.frames += 1;

        let root = div()
            .relative()
            .size_full()
            .bg(rgb(0x020617))
            .on_mouse_move(|_: &MouseMoveEvent, _, _| {})
            .flex()
            .flex_col()
            // --- A: the failing shape, verbatim --------------------------------
            // Stretched, `py`/`px` both-side padding, plain children. This is
            // what the map chrome's north strip became.
            .child(band("A: stretch + px/py + flat children", |s| {
                s.top(px(0.0)).px(px(12.0)).py(px(18.0))
            }))
            // --- B: no padding at all -----------------------------------------
            .child(band("B: stretch, no padding", |s| s.top(px(72.0))))
            // --- C: top_zone's exact shape ------------------------------------
            // `.px()` only, plus a `flex_1` child — the one shape in the
            // console that is KNOWN to lay its children out.
            .child(band("C: stretch + px + flex_1 child", |s| {
                s.top(px(144.0))
                    .px(px(10.0))
                    .child(div().flex_1().flex_shrink_0())
            }))
            // --- D: single-side padding ---------------------------------------
            // `pl` alone killed a strip outright in the console.
            .child(band("D: stretch + pl only", |s| {
                s.top(px(216.0)).pl(px(20.0))
            }))
            // --- E: not absolute at all ---------------------------------------
            // A plain in-flow child of a relative parent: the control case.
            .child(
                div()
                    .mt(px(288.0))
                    .h(px(64.0))
                    .w_full()
                    .bg(rgb(NIGHT))
                    .flex()
                    .flex_row()
                    .items_center()
                    .px(px(12.0))
                    .child(
                        div()
                            .text_color(rgb(INK))
                            .child("E: in-flow, not absolute (control)"),
                    )
                    .child(div().w(px(40.0)).h(px(28.0)).bg(rgb(AMBER))),
            )
            // --- F: a full-bleed absolute sibling painted FIRST ---------------
            // The console's map surface is `.absolute().top(0).left(0).right(0)
            // .bottom(0)` and is the first child of the root, exactly like
            // this. If GPUI orders absolute siblings' CHILDREN differently
            // from their backgrounds, the strip's background would still land
            // (which is what the console shows — a 562-unit scrim delta) while
            // its children vanished behind the map.
            .child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .bottom(px(0.0))
                    .bg(rgb(0x1b3a5c)),
            )
            .child(band("F: full-bleed sibling first", |s| {
                s.top(px(360.0)).px(px(12.0))
            }))
            // --- G: same, but the strip is added LAST -------------------------
            .child(band("G: full-bleed first, strip last", |s| {
                s.top(px(432.0)).px(px(12.0))
            }))
            // --- H: the console's exact path -----------------------------------
            .children(chrome_vec(504.0, 18.0, 64.0))
            // --- I: py larger than half the height ----------------------------
            // A negative content box. The console's last north strip was
            // `h(44)` with `py(26)`, which leaves -8px for its children.
            .children(chrome_vec(576.0, 26.0, 44.0))
            .child(
                div()
                    .absolute()
                    .bottom(px(8.0))
                    .left(px(12.0))
                    .text_color(rgb(0x8c9bae))
                    .text_size(px(11.0))
                    .child(format!("frames {}", self.frames)),
            );
        root.into_any_element()
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(760.0), px(420.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                app_id: Some("gpui-layout-repro".to_string()),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Repro { frames: 0 }),
        )
        .expect("window opens");
    });
}
