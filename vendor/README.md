# GPUI Base window rebinding patch

`gpui-base/` is the crates.io source of `gpui-base 0.7.0` (Apache-2.0).
It is patched through Cargo so the published GPUI Kit dependencies use the same
editing engine. No registry cache is modified.

The change in `src/input/base/state.rs` separates window focus/activation
subscriptions from engine subscriptions and adds `bind_window`. Rendering in a
new window automatically rebinds these observers while retaining text,
selections and undo history. Vyber also binds hidden inputs during transfers.
Remove this patch when the pinned upstream release supplies equivalent behavior.
