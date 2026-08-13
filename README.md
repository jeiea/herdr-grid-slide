# herdr-move-pane

Move the current Herdr pane cyclically to the adjacent tab or workspace, or navigate panes in visual
order across container boundaries. A moved pane splits the destination on the right at a 50:50 ratio
and keeps focus.

Workspace moves target the active tab of the adjacent workspace. Tab moves stay within the current
workspace. Both use the visible `number` order and wrap at either end.

Directional focus uses Herdr's geometric neighbor within the current tab. At a horizontal edge it
wraps to the previous or next tab; at a vertical edge it wraps to the previous or next workspace's
active tab. Boundary panes follow reading order from top-left to bottom-right.

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
