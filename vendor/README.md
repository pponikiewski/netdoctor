# vendor/

Third-party crates patched in place through `[patch.crates-io]` in
`Cargo.toml`. Each is the published crate, unchanged apart from the lines
marked `NETDOC PATCH`.

## winit 0.30.13

Source: crates.io `winit-0.30.13`, Apache-2.0 (its `LICENSE` is kept).

Changed: `src/platform_impl/windows/monitor.rs`, `MonitorHandle::name`,
`native_identifier` and `size`. Upstream calls `GetMonitorInfoW` and unwraps
the result. When a monitor is unplugged, or the layout changes on waking or
docking, a handle taken a moment earlier fails with
`ERROR_INVALID_MONITOR_HANDLE` (1461) and the unwrap panics. egui-winit asks
for the current monitor's size on every frame (`update_viewport_info`), and
the release build aborts on panic, so the window simply vanished. Seen in
`crash.log` on 1.5.0. Upstream issue: rust-windowing/winit#3258.

The patched methods answer "no name" and a size of 0x0 for a handle that has
gone, the way `position` already does upstream. The next frame asks again and
gets the new monitor.

When to remove: when eframe moves to a winit release where these methods no
longer unwrap. Check `MonitorHandle::size` in the new winit's
`platform_impl/windows/monitor.rs`, then delete this folder and the
`[patch.crates-io]` entry. Cargo warns `patch ... was not used` once the
version no longer matches, which is the reminder.

Changed: `src/platform_impl/windows/window.rs`, `Window::request_redraw`.
Windows sends no `WM_PAINT` to a hidden window, so upstream's `RedrawWindow`
asks for a `RedrawRequested` that never arrives. eframe 0.29 sets
`ControlFlow::Poll` while it waits for that event (`check_redraw_requests`),
and the event loop then spins without sleeping until some unrelated input,
typically a mouse move, makes eframe reset it to `Wait`. The app lives hidden
in the tray, and there it was measured burning up to 80% of a core for as
long as the mouse stayed still. The patch queues a `WM_PAINT` for a hidden
window; the `WM_PAINT` handler turns it into the event either way.

When to remove: when winit delivers `RedrawRequested` for hidden windows on
Windows, or eframe stops polling while it waits for one. Check
`request_redraw` in the new winit's `platform_impl/windows/window.rs` and
`check_redraw_requests` in the new eframe's `native/run.rs`.
