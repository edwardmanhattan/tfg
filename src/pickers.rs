//! The console's two pickers: the calendar and the time grid.
//!
//! THEY ARE COMPONENTS, not a modal's private code. The scenario composer
//! grew a calendar and a time grid, and the session wizard wanted the same
//! two controls; a second copy of a month grid is a second thing to fix
//! when the type scale moves, and the console's whole argument for having a
//! type scale at all is that there is ONE of everything.
//!
//! So each picker is a function that takes what it draws and what it means:
//! the value it holds, and what a click means. It knows nothing about the
//! scenario composer, the session wizard, or the shell — which is what makes
//! it usable by both, and by whatever comes next.
//!
//! AND THEY ARE POPUPS. A calendar drawn inline pushes the panel down every
//! time it opens, and a panel that moves under the operator's cursor is a
//! panel that has lost the field they were in. A picker that opens UNDER the
//! trigger leaves the form where it was and puts the answer next to the
//! question.
//!
//! The one thing a picker needs from its host is WHERE to open, and that is
//! the host's business rather than the picker's: the shell knows where the
//! trigger is, and hands the popup a position.

use crate::calendar;
use crate::ink::rgba;
use crate::view::{
    self as view, BODY_SILVER, CUT_GREY, DEEP_WELL, HAIRLINE_SLATE, MAP_INK, RADAR_CYAN,
};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, div, px,
};
use std::rc::Rc;

/// A calendar cell's width.
///
/// The grid is SEVEN cells wide, so this is a share of the popup's width
/// rather than a number of its own: the popup is as wide as the grid, and
/// the grid is as wide as seven days.
pub const CELL_W: f32 = 30.0;
/// A calendar cell's height.
pub const CELL_H: f32 = 24.0;

/// The calendar: a month, six weeks, seven days.
///
/// `month` is the month being shown, `selected` the day the field holds
/// (if any), `on_shift` walks the month backwards or forwards, and
/// `on_pick` hands a day back.
pub fn date_picker<V: 'static>(
    cx: &mut Context<V>,
    month: calendar::Month,
    selected: Option<u32>,
    on_shift: impl Fn(&mut V, bool, &mut Context<V>) + 'static,
    on_pick: impl Fn(&mut V, u32, &mut Context<V>) + 'static,
) -> impl IntoElement {
    let grid = month.grid();
    // The callbacks are SHARED, not moved: a grid has one handler per cell
    // and every cell answers the same question.
    let on_shift = Rc::new(on_shift);
    let on_pick = Rc::new(on_pick);
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .p(px(7.0))
        .rounded(px(view::CONTROL_RADIUS))
        .border(px(1.0))
        .border_color(rgba(HAIRLINE_SLATE))
        .bg(rgba(DEEP_WELL));
    // The month, and the two arrows that walk it.
    col = col.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(6.0))
            .child(
                div()
                    .id("cal-prev")
                    .cursor_pointer()
                    .on_click({
                        let on_shift = on_shift.clone();
                        cx.listener(move |view, _, _, cx| on_shift(view, false, cx))
                    })
                    .child(arrow("icons/chevron-left.svg")),
            )
            .child(
                div()
                    .font_family(view::FACE_HUMAN)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(view::pt(view::DENSE_PT)))
                    .text_color(rgba(MAP_INK))
                    .child(month.label()),
            )
            .child(
                div()
                    .id("cal-next")
                    .cursor_pointer()
                    .on_click({
                        let on_shift = on_shift.clone();
                        cx.listener(move |view, _, _, cx| on_shift(view, true, cx))
                    })
                    .child(arrow("icons/chevron-right.svg")),
            ),
    );
    // The weekday heads. Monday first: the console's weeks run that way
    // everywhere else, and a calendar that starts on Sunday is a second
    // thing to learn.
    let heads = ["M", "T", "W", "T", "F", "S", "S"];
    let mut head = div().flex().flex_row().gap(px(2.0));
    for h in heads {
        head = head.child(
            div()
                .w(px(CELL_W))
                .flex()
                .justify_center()
                .font_family(view::FACE_MACHINE)
                .text_size(px(view::pt(9.0)))
                .text_color(rgba(CUT_GREY))
                .child(h.to_string()),
        );
    }
    col = col.child(head);
    for week in &grid.weeks {
        let mut row = div().flex().flex_row().gap(px(2.0));
        for cell in week {
            match cell {
                Some(day) => {
                    let on = selected == Some(*day);
                    let day = *day;
                    row = row.child(
                        div()
                            .id(("cal-day", day as usize))
                            .w(px(CELL_W))
                            .h(px(CELL_H))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(2.0))
                            .border(px(1.0))
                            .border_color(rgba(if on { RADAR_CYAN } else { HAIRLINE_SLATE }))
                            .font_family(view::FACE_MACHINE)
                            .text_size(px(view::pt(10.0)))
                            .text_color(rgba(if on { MAP_INK } else { BODY_SILVER }))
                            .cursor_pointer()
                            .on_click({
                                let on_pick = on_pick.clone();
                                cx.listener(move |view, _, _, cx| on_pick(view, day, cx))
                            })
                            .child(format!("{day}")),
                    );
                }
                None => {
                    row = row.child(div().w(px(CELL_W)).h(px(CELL_H)));
                }
            }
        }
        col = col.child(row);
    }
    col
}

/// The time grid: the hours, then the minutes.
///
/// The HOURS are a grid of twenty-four, six to a row, because an hour is a
/// shape on a dial and a column of twenty-four would be a list. The
/// MINUTES are five to a step, in two rows, because the quarter-hour is what
/// an exercise is timed on and the field still takes any minute the operator
/// types.
///
/// `on_pick` hands back the hour AND the minute, because the two are one
/// time: picking an hour keeps the minute the field already held, and
/// picking a minute keeps the hour. The last argument says WHICH grid was
/// used, because an hour and a minute are two acts and the host may want to
/// close the popup after one of them and not the other.
pub fn time_picker<V: 'static>(
    cx: &mut Context<V>,
    hour: Option<u32>,
    minute: Option<u32>,
    on_pick: impl Fn(&mut V, u32, u32, bool, &mut Context<V>) + 'static,
) -> impl IntoElement {
    let on_pick = Rc::new(on_pick);
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .p(px(7.0))
        .rounded(px(view::CONTROL_RADIUS))
        .border(px(1.0))
        .border_color(rgba(HAIRLINE_SLATE))
        .bg(rgba(DEEP_WELL));
    col = col.child(caption("HOURS"));
    for row in 0..4u32 {
        let mut line = div().flex().flex_row().gap(px(2.0));
        for column in 0..6u32 {
            let h = row * 6 + column;
            let on = hour == Some(h);
            let minute = minute.unwrap_or(0);
            line = line.child(
                div()
                    .id(("time-hour", h as usize))
                    .w(px(CELL_W))
                    .h(px(CELL_H))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(2.0))
                    .border(px(1.0))
                    .border_color(rgba(if on { RADAR_CYAN } else { HAIRLINE_SLATE }))
                    .font_family(view::FACE_MACHINE)
                    .text_size(px(view::pt(10.0)))
                    .text_color(rgba(if on { MAP_INK } else { BODY_SILVER }))
                    .cursor_pointer()
                    .on_click({
                        let on_pick = on_pick.clone();
                        cx.listener(move |view, _, _, cx| on_pick(view, h, minute, false, cx))
                    })
                    .child(format!("{h:02}")),
            );
        }
        col = col.child(line);
    }
    col = col.child(caption("MINUTES"));
    for row in 0..2u32 {
        let mut line = div().flex().flex_row().gap(px(2.0));
        for column in 0..6u32 {
            let m = (row * 6 + column) * 5;
            let on = minute == Some(m);
            let hour = hour.unwrap_or(0);
            line = line.child(
                div()
                    .id(("time-minute", m as usize))
                    .w(px(CELL_W))
                    .h(px(CELL_H))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(2.0))
                    .border(px(1.0))
                    .border_color(rgba(if on { RADAR_CYAN } else { HAIRLINE_SLATE }))
                    .font_family(view::FACE_MACHINE)
                    .text_size(px(view::pt(10.0)))
                    .text_color(rgba(if on { MAP_INK } else { BODY_SILVER }))
                    .cursor_pointer()
                    .on_click({
                        let on_pick = on_pick.clone();
                        cx.listener(move |view, _, _, cx| on_pick(view, hour, m, true, cx))
                    })
                    .child(format!("{m:02}")),
            );
        }
        col = col.child(line);
    }
    col
}

/// A grid's caption: the name of the thing below it, in machine caps.
///
/// The caption is a CAPTION and not a heading: it names a section of the
/// popup, in the machine face, at the size the machine face is read at.
fn caption(text: &'static str) -> impl IntoElement {
    div()
        .font_family(view::FACE_MACHINE)
        .text_size(px(view::pt(9.0)))
        .text_color(rgba(CUT_GREY))
        .child(text.to_string())
}

/// One of the month's two arrows.
///
/// A DRAWN chevron and not a `‹` character: the console's icons are SVG,
/// drawn in one weight, and a text glyph among drawn icons is a different
/// voice saying the same thing.
fn arrow(path: &'static str) -> impl IntoElement {
    div()
        .w(px(22.0))
        .h(px(22.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(view::CONTROL_RADIUS))
        .border(px(1.0))
        .border_color(rgba(HAIRLINE_SLATE))
        .bg(rgba(DEEP_WELL))
        .child(crate::ink::icon(path, 9.0, 9.0, CUT_GREY))
}
