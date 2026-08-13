# herdr-move-pane

Move the current Herdr pane cyclically to the adjacent tab or workspace, or navigate panes in visual
order across container boundaries. A moved pane splits the destination on the right at a 50:50 ratio
and keeps focus.

Workspace moves target the active tab of the adjacent workspace. Tab moves stay within the current
workspace. Both use the visible `number` order and wrap at either end.

Directional focus preserves the cross-axis position across panes where possible. At a horizontal
edge it wraps to the previous or next tab; at a vertical edge it wraps to the previous or next
workspace's active tab. The preferred position carries across boundaries, with reading order used to
break ties.

The focus position is stored in `HERDR_PLUGIN_STATE_DIR/focus-anchor.json`. Focus actions do not use
a lock file. Each successful action writes a temporary file and atomically replaces the state, which
prevents partially written JSON but does not order overlapping processes. Either process may replace
the anchor after the other focuses, and inputs that read the same snapshot may choose the same target.
Each action uses the snapshot's live focused pane instead of its inherited pane context, reducing
stale-context errors without promising exact ordering for simultaneous inputs.

Actions use `HERDR_SOCKET_PATH` directly for both the session snapshot and the resulting pane
operation, avoiding an additional Herdr CLI process per key press.

## Development

Requires Rust 1.97 or later.

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
herdr plugin link .
```

The plugin exposes these actions:

- `jeiea.move-pane.to-next-workspace`
- `jeiea.move-pane.to-previous-workspace`
- `jeiea.move-pane.to-next-tab`
- `jeiea.move-pane.to-previous-tab`
- `jeiea.move-pane.focus-left`
- `jeiea.move-pane.focus-down`
- `jeiea.move-pane.focus-up`
- `jeiea.move-pane.focus-right`
