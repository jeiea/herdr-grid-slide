# herdr-move-pane

Move the current Herdr pane cyclically to the adjacent tab or workspace, navigate panes in visual
order across container boundaries, add a pane without squeezing the ones around it, or lay a whole
tab out as an even grid. A moved pane splits the destination on the right at a 50:50 ratio and keeps
focus.

Workspace moves target the active tab of the adjacent workspace. Tab moves stay within the current
workspace. Both use the visible `number` order and wrap at either end.

Directional focus preserves the cross-axis position across panes where possible. At a horizontal
edge it wraps to the previous or next tab; at a vertical edge it wraps to the previous or next
workspace's active tab. The preferred position carries across boundaries, with reading order used to
break ties.

`split-pane` splits the focused pane and then evens out the row or column the new pane lands in.
Herdr's own split halves the focused pane, so repeated splits leave the newest panes ever narrower;
this action gives every slot of that run an equal share instead, and focuses the pane it created.
It splits along the direction the tab already grows in, and falls back to the shape of the focused
pane when the tab mixes directions or holds a single pane: wider than twice its height splits to
the right, anything else splits downwards, which accounts for terminal cells being about twice as
tall as they are wide. A subtree running the other way counts as one slot, so evening out a row
leaves the column inside it alone.

`balance` lays the whole tab out as an even two-axis grid, keeping the panes in reading order. The
column count is roughly `sqrt(panes × width / (2 × height))`, the same cell correction as above,
clamped between one and the pane count, and the last row is allowed to come up short. Each run does
the least the tab needs: one that is already the right grid is only resized, one whose panes sit in
the wrong cells is put right with swaps, and only a tab shaped differently is rebuilt. Rebuilding
parks every pane but the first in a scratch tab and brings them back one at a time, because Herdr
declines to move a pane within its own tab; the terminals carry across intact and the emptied
scratch tab disappears with the last move. Whether it finishes or fails, `balance` puts the focus
back on the pane it started from, best effort. Running `balance` again on its own result only
reapplies the target ratios.

Both actions stop with an error on a zoomed tab rather than unzooming it, so a failure part-way
through cannot leave the tab zoomed out. Herdr clamps split ratios to [0.1, 0.9], so a run of more
than ten same-direction slots cannot be made exactly even; rebuilt grids join panes as balanced
halves and stay clear of the clamp. A rebuild that fails part-way tries to bring the parked panes
back beside the first one, preferring to keep every terminal over restoring the previous
arrangement, and names any pane it could not bring back along with the tab holding it. Rebuilding
also moves panes through another tab, so the layout visibly churns while it runs.

The focus position is stored in `HERDR_PLUGIN_STATE_DIR/focus-anchor.json`. Focus actions do not use
a lock file. Each successful action writes a temporary file and atomically replaces the state, which
prevents partially written JSON but does not order overlapping processes. Either process may replace
the anchor after the other focuses, and inputs that read the same snapshot may choose the same target.
Each action uses the snapshot's live focused pane instead of its inherited pane context, reducing
stale-context errors without promising exact ordering for simultaneous inputs. `split-pane` and
`balance` take no lock either; they check the session snapshot against the exported layout before
touching anything, and a rebuild re-reads the tab before resizing it, which catches a tab that
changed underneath them without serialising simultaneous inputs.

Actions use `HERDR_SOCKET_PATH` directly for both the session snapshot and the resulting pane
operation, avoiding an additional Herdr CLI process per key press.

## Install

Requires Herdr 0.8 or later and `curl`. Rust is not required. The installer uses `sha256sum` on Linux
or the preinstalled `shasum` on macOS to verify the download.

```sh
herdr plugin install jeiea/herdr-move-pane
```

Herdr clones the repository and runs the plugin's build command. The command downloads the raw
binary for the plugin version, verifies it against the release's `SHA256SUMS`, and only then replaces
the installed binary. Re-running the install command updates to the version declared by the fetched
plugin source.

To pin a released version, install its Git tag:

```sh
herdr plugin install --ref v0.1.0 jeiea/herdr-move-pane
```

Supported targets:

- macOS on Apple Silicon (`aarch64-apple-darwin`)
- macOS on Intel (`x86_64-apple-darwin`)
- Linux ARM64 (`aarch64-unknown-linux-musl`)
- Linux x86-64 (`x86_64-unknown-linux-musl`)

## Development

Requires Rust 1.97 or later.

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
herdr plugin link .
```

When `target/release/herdr-move-pane` exists, the plugin build command copies that local binary into
`bin/`. Otherwise it follows the same verified release-download path used by installations.

The plugin exposes these actions:

- `jeiea.move-pane.to-next-workspace`
- `jeiea.move-pane.to-previous-workspace`
- `jeiea.move-pane.to-next-tab`
- `jeiea.move-pane.to-previous-tab`
- `jeiea.move-pane.focus-left`
- `jeiea.move-pane.focus-down`
- `jeiea.move-pane.focus-up`
- `jeiea.move-pane.focus-right`
- `jeiea.move-pane.split-pane`
- `jeiea.move-pane.balance`

Release maintenance and the checks that remain after making this repository public are documented in
[`docs/releasing.md`](docs/releasing.md).

## License

[MIT](LICENSE)
