# GPUI 0.2.2 dispatch facts

The facts below were re-derived by reading `gpui-0.2.2`'s source during the
docked-sidebar slice, at a cost of about eight tool calls. They are not
obvious, they contradict the shape of the API, and getting one of them
wrong is a silent UI bug rather than a compile error. So they are written
down here, once, with the source they came from.

Paths are relative to
`~/.cargo/registry/src/index.crates.io-*/gpui-0.2.2/src/`.

## The scroll hit test collects EVERY hitbox under the pointer

`window.rs:775` — `Window::hit_test`:

```rust
for hitbox in self.hitboxes.iter().rev() {     // topmost first
    let bounds = hitbox.bounds.intersect(&hitbox.content_mask.bounds);
    if bounds.contains(&position) {
        hit_test.ids.push(hitbox.id);           // EVERY containing hitbox
        if !set_hover_hitbox_count
            && hitbox.behavior == HitboxBehavior::BlockMouseExceptScroll
        {
            hit_test.hover_hitbox_count = hit_test.ids.len();
            set_hover_hitbox_count = true;
        }
        if hitbox.behavior == HitboxBehavior::BlockMouse {
            break;                              // the ONLY thing that stops it
        }
    }
}
```

The list is NOT the path to the pointer. It is every hitbox whose bounds
contain it, topmost first, and the walk only stops at a hitbox whose
behaviour is `BlockMouse`.

## Two predicates, and they answer different questions

| predicate | source | question it answers |
|---|---|---|
| `HitboxId::is_hovered(window)` | `window.rs:499` | is this hitbox on the HOVER PATH? — membership in `ids[..hover_hitbox_count]`, which stops at the first `BlockMouseExceptScroll` |
| `HitboxId::should_handle_scroll(window)` | `window.rs:511` | is the pointer over this hitbox AT ALL? — `ids.contains(&self)`, with no blocker logic |

`BlockMouseExceptScroll` (what the island panels use) sets
`hover_hitbox_count` but does **not** remove the hitboxes beneath it from
`ids`. So a full-window background element's `on_scroll_wheel` fires for a
wheel that began over chrome painted on top of it.

**This bit us**: the console's map surface is a full-window hitbox, so a
wheel over the docked column reached the map's zoom handler as well as the
column's scroll handler. The map zoomed while the operator was scrolling a
panel.

## The bubble phase runs deepest-first, and `stop_propagation` ends it

`window.rs:3695` — `Window::dispatch_event`: the capture phase walks
`mouse_listeners` in registration order, then the bubble phase walks
`mouse_listeners.iter_mut().rev()`. Listeners register during prepaint, so
the bubble phase calls the LAST-PAINTED element first and the
FIRST-PAINTED one last.

`App::stop_propagation` (`app.rs:1712`) sets `propagate_event = false`,
and the bubble loop breaks on it. `Context<T>` derefs to `App`, so it is
reachable from a `cx.listener` closure.

⇒ **A chrome element painted AFTER a background element can swallow the
background's wheel** by calling `cx.stop_propagation()` in its own wheel
handler. The background must be painted first for this to work; in the
console the map surface is the first child of the root, so its listener is
registered first and therefore called LAST.

## A div with only a wheel handler is still a pointer target

`elements/div.rs:1707` — `should_insert_hitbox` includes
`!self.scroll_wheel_listeners.is_empty()`. So adding a wheel handler to a
div gives that div a hitbox, which changes what the pointer is over.

## The rule to remember

> A gesture belongs to the thing UNDER the pointer, and only that thing.
> Paint order decides who is under whom; the hit test does not.

If a background handler must not see a gesture that began over the
chrome, the chrome stops the propagation. It cannot be argued from the
hit test, because the hit test will hand the gesture to both.
