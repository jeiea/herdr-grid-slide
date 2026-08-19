# herdr-move-pane

Move the current Herdr pane cyclically to the adjacent tab or workspace, reorder the workspaces
themselves, navigate panes in visual order across container boundaries, add a pane without squeezing
the ones around it, or lay a whole tab out as an even grid. A moved pane splits the destination on
the right at a 50:50 ratio and keeps focus. When a pane exits, the plugin automatically balances the
tab it belonged to.

Workspace moves target the active tab of the adjacent workspace. Tab moves stay within the current
workspace. Both use the visible `number` order and wrap at either end.

`move-workspace` reorders the workspaces themselves instead of moving a pane: it shifts the active
workspace one position through the visible `number` order, wrapping from either end to the other.
Herdr keeps the active and selected workspaces by identity, so focus stays on the moved workspace.
With a single workspace the action succeeds without side effects.

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

`new-pane` chooses the same adaptive split direction, creates the pane at 50:50, and then balances
the whole tab while keeping the new pane focused. It does not even out the new pane's local row or
column first; the whole-tab balance replaces that intermediate resize.

The `pane.exited` hook balances the exited pane's tab from the event context, even when another tab
is active. It restores the session's global focus after balancing; if the snapshot has no global
focus, it uses the first surviving pane in the target tab's reading order. If the last pane removed
the tab, or only one pane survives, the hook does nothing successfully.

`balance` lays the whole tab out as an even two-axis grid, keeping the panes in reading order. The
ideal column count is `sqrt(panes × width / (2 × height))`, the same cell correction as above. If its
floor or ceiling divides the pane count, the closest such divisor is used (the larger on a tie);
otherwise the ideal is rounded normally, so the last row may come up short. The result stays clamped
between one and the pane count. Each run does the least the tab needs: one that is already the right
grid is only resized, one whose panes sit in the wrong cells is put right with swaps, and only a tab
shaped differently is rebuilt. Rebuilding parks every pane but the first in a scratch tab and brings
them back one at a time, because Herdr declines to move a pane within its own tab; the terminals carry
across intact and the emptied scratch tab disappears with the last move. Whether it finishes or
fails, `balance` puts the focus back on the pane it started from, best effort. Running `balance` again
on its own result only reapplies the target ratios.

The three explicit layout actions and automatic exit balance stop with an error on a zoomed tab
rather than unzooming it, so a failure part-way through cannot leave the tab zoomed out. Herdr
clamps split ratios to [0.1, 0.9], so a run of more than ten same-direction slots cannot be made
exactly even; rebuilt grids join panes as balanced halves and stay clear of the clamp. A rebuild
that fails part-way tries to bring the parked panes back beside the first one, preferring to keep
every terminal over restoring the previous arrangement, and names any pane it could not bring back
along with the tab holding it. Rebuilding also moves panes through another tab, so the layout visibly
churns while it runs.

The focus position is stored in `HERDR_PLUGIN_STATE_DIR/focus-anchor.json`. Focus actions do not use
a lock file. Each successful action writes a temporary file and atomically replaces the state, which
prevents partially written JSON but does not order overlapping processes. Either process may replace
the anchor after the other focuses, and inputs that read the same snapshot may choose the same target.
Each action uses the snapshot's live focused pane instead of its inherited pane context, reducing
stale-context errors without promising exact ordering for simultaneous inputs. `split-pane`,
`new-pane`, and `balance` take no lock either; they check the session snapshot against the exported
layout before touching anything. Pane creation re-reads the affected tab before balancing it, and a
rebuild reads it once more before resizing, which catches a tab that changed underneath them without
serialising simultaneous inputs.

Herdr handles the exit before servicing the hook's socket request, so the hook reads one fresh
snapshot with the exited pane already removed. It uses the event's `HERDR_WORKSPACE_ID`,
`HERDR_TAB_ID`, and `HERDR_PANE_ID`, not the active tab context. There is no delay or retry: if that
single snapshot still contains the exited pane, the hook fails explicitly instead of balancing a
stale layout. Overlapping exits or other layout changes can therefore fail the same consistency
checks as a manual balance; the next exit is a separate run, not a retry of the failed one.

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
- `jeiea.move-pane.move-workspace-next`
- `jeiea.move-pane.move-workspace-previous`
- `jeiea.move-pane.focus-left`
- `jeiea.move-pane.focus-down`
- `jeiea.move-pane.focus-up`
- `jeiea.move-pane.focus-right`
- `jeiea.move-pane.split-pane`
- `jeiea.move-pane.new-pane`
- `jeiea.move-pane.balance`

Release maintenance and the checks that remain after making this repository public are documented in
[`docs/releasing.md`](docs/releasing.md).

## License

[MIT](LICENSE)
