# gpui 0.2.2, with the Wayland `xdg_wm_base` floor lowered to 1

This directory is `gpui` 0.2.2 exactly as published on crates.io, plus the
change below. It is vendored because the console must start on a WSLg desktop,
and WSLg's Weston is one of the compositors upstream assumes away.

## The change

`src/platform/linux/wayland/client.rs`, `Globals::new`:

```diff
-            wm_base: globals.bind(&qh, 2..=5, ()).unwrap(),
+            wm_base: globals.bind(&qh, 1..=5, ()).unwrap(),
```

## Why

`wayland_client::globals::GlobalList::bind` returns
`BindError::UnsupportedVersion` whenever the version the compositor advertised
is below the floor of the requested range — the range's maximum is only used to
cap what gets bound. Upstream asks for `2..=5`, so a compositor advertising
`xdg_wm_base` version 1 fails the bind, and upstream unwraps it:

```
thread 'main' (221) panicked at gpui-0.2.2/src/platform/linux/wayland/client.rs:151:51:
called `Result::unwrap()` on an `Err` value: UnsupportedVersion
```

WSLg (Debian 13 on WSL, `WSLg 1.0.73`) advertises exactly that: `xdg_wm_base`
version 1, because its Weston still exports xdg-shell from before the v2
semantics change. Everything else WSLg offers is new enough — it advertises
`wl_compositor` and `wl_seat` past GPUI's own floors — so the base version is
the only thing that stands between the binary and a window.

## Why a floor of 1 is safe

GPUI uses `xdg_wm_base` for two things, both version 1: `get_xdg_surface` and
`pong` (on `ping`). The `xdg_surface` and `xdg_toplevel` it creates inherit the
base version, and the requests it then sends are all version 1 —
`ack_configure`, `set_window_geometry`, and the toplevel setters. The only
version-gated thing in the Wayland backend, `wl_surface.preferred_buffer_scale`
(v6), is already guarded by `surface.version()` in `window.rs`. Binding at 1
therefore costs a compositor that advertises more nothing at all, and the
`xdg_toplevel.wm_capabilities` event (v5) simply never arrives.

## Re-applying on a version bump

`gpui` is pinned exact (`=0.2.2`) in the root `Cargo.toml` because 0.2.x is what
U1–U8 measured (ADR-0020) and a minor bump is a new blade-graphics with a new
shader and surface model. When that pin does move, re-check upstream's floor in
`Globals::new`: if it is still `2..=5`, re-apply the diff above to the new
crate, and update this file.

The X11 backend is unaffected: GPUI selects Wayland when `WAYLAND_DISPLAY` is
set, and X11 (through XWayland, which WSLg also provides) when it is not.
