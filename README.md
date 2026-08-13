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

The focus position is stored in `HERDR_PLUGIN_STATE_DIR/focus-anchor.json`. Focus actions use a lock
file in the same directory so rapidly invoked plugin processes update that state in order.

## Development

Requires Deno 2.9 or later.

```sh
deno task check
deno task build
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
