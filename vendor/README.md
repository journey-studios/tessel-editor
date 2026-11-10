# Vendored crates

Patched copies of third-party crates, wired in by `[patch.crates-io]` in the root `Cargo.toml`
and excluded from the workspace (`exclude = ["vendor"]`), so `fmt`/`clippy` do not touch them.

## `gpui-pre-windows` 0.3.11 (Apache-2.0, `LICENSE-APACHE` kept)

Upstream: `gpui-pre-windows` 0.3.11 from crates.io, copied unchanged except one line in
`src/directx_renderer.rs`:

- `DirectComposition::new` calls `CreateTargetForHwnd(hwnd, false)` instead of `true`.

Why: with `topmost = true` the swap chain's composition tree is drawn above every child window,
so the native WebView2 view (the monitor, `viewport.rs`) was invisible even though its page was
drawing (checked through the WebView2 DevTools protocol: the canvas had pixels and the page
screenshot was correct, while a screen capture of the window showed only the GPUI background).
With `false` the child window is drawn over the GPUI content, which is the model the app already
assumes everywhere ("the native WebView paints above any GPUI element", `Workflow::has_dialog`).

Remove the patch when the upstream renderer composes a hosted WebView2 correctly; keep the
version equal to the one `gpui-kit` resolves, or Cargo ignores the patch.
