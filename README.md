# herdr-move-pane

Move the current Herdr pane cyclically to the adjacent tab or workspace, create a tab or workspace
immediately after the current one, move the pane to a new tab or workspace, reorder workspaces,
navigate panes in visual order across container boundaries, split a pane, or lay a tab out as an
even grid. Pane moves split an existing destination on the right at a 50:50 ratio and keep focus.
Move, split, and new pane actions do not balance layouts themselves, except that a successful
tab-scoped move starts automatic balance before returning. It balances the destination if that tab
is still focused after the balance lock is acquired.

Workspace-scoped pane moves target the active tab of the adjacent workspace and use visible
workspace `number` order. Tab-scoped pane moves stay in the current workspace and follow displayed
tab order. Both wrap at either end. `move-workspace` instead moves the active workspace itself
through workspace `number` order. Directional focus preserves its cross-axis position where possible
and wraps across tabs or workspaces at an edge.

`new-tab` creates a focused tab with its default shell pane in the current workspace and places it
immediately after the current tab. It leaves the current pane in place and does not override the new
tab's working directory, name, command, or environment. If the current tab was already last, Herdr
creates the new tab in the right place and no reorder request is needed.

`new-workspace` creates a focused workspace with its default shell pane and places it immediately
after the current workspace in visible `number` order. It leaves the current pane in place and does
not override the new workspace's working directory, name, command, or environment. If the current
workspace was already last, Herdr creates the new workspace in the right place and no reorder
request is needed.

`to-new-tab` moves the focused pane into a focused new tab in the same workspace and places that tab
immediately after its source. If the source was already the last tab, the new tab is already in the
right place and no reorder request is needed. Moving the source tab's only pane would close that tab,
so the action ignores that case without changing the session. If Herdr declines the pane move, the
action likewise stops without a reorder request.

`to-new-workspace` moves the focused pane into a focused new workspace and places that workspace
immediately after its source in visible `number` order. If the source was already the last
workspace, the new workspace is already in the right place and no reorder request is needed. Moving
the source workspace's only pane would close that workspace, so the action ignores that case without
changing the session. Moving a tab's only pane is allowed when another tab remains in the source
workspace. If Herdr declines the pane move, the action likewise stops without a reorder request.

`split-pane` and `new-pane` are aliases. They split the focused pane at 50:50 along the direction
the tab already grows in. For mixed or single-pane layouts, a pane wider than twice its height splits
right; all others split down.

`pane.focused` and successful tab-scoped moves enter automatic balance. The plugin records the latest
entered tab and its pane set in `HERDR_PLUGIN_STATE_DIR/balance-focus.json`; another pane focus in
that same tab with the same panes does nothing, while a focus that arrives with a new or closed pane
-- such as the one `new-pane` creates -- balances the tab again. A tab-scoped move ignores a matching
cached pane set once when the first snapshot after locking still focuses its expected destination.
If that snapshot focuses another tab, the expected destination is discarded and the usual latest
focus and cache rules apply. This makes an externally moved pane balance its destination when entered
and leaves the source to be balanced when the user later enters it. Hook processes share a file lock,
read the latest session snapshot after acquiring it, and keep following newer tab focus until the
observed tab is settled.
Internal focus events from swaps, scratch-tab moves, and rebuilding therefore collapse into the same
run instead of starting recursive work. With no state yet, the first observed focus is treated as a
tab entry because Herdr does not provide the previous tab.

If a tab-scoped pane move succeeds but automatic balance fails, the pane remains moved and the action
fails with `pane moved, but automatic balance failed: ...`.

If `to-new-tab` moves the pane but Herdr omits the created tab ID, or the subsequent tab reorder
fails, the action fails with an error that says the pane move already completed. The pane remains in
the new tab; the tab may remain at the end of its workspace.

If `new-tab` needs to reorder the created tab but cannot obtain its ID from Herdr's response, or if
the subsequent reorder fails, the action fails with an error that says tab creation already
completed. The new tab remains focused and may remain at the end of its workspace.

If `new-workspace` needs to reorder the created workspace but cannot obtain its ID from Herdr's
response, or if the subsequent reorder fails, the action fails with an error that says workspace
creation already completed. The new workspace remains focused and may remain at the end of the
session.

If `to-new-workspace` moves the pane but Herdr omits the created workspace ID, or the subsequent
workspace reorder fails, the action fails with an error that says the pane move already completed.
The pane remains in the new workspace; that workspace may remain at the end of the session.

Automatic balance quietly skips a missing, single-pane, or zoomed tab. It records an attempted tab
before changing its layout, so a failure does not loop on ordinary same-tab pane focus; leaving and
re-entering the tab retries it. On failure the automatic path never sends `pane.focus`. After a
successful focus-changing operation it restores the entry pane only when the latest snapshot still
matches the focus that operation itself was expected to produce. Before each swap or scratch move it
also verifies that its target tab is still focused. After every attempt it reads the latest focus
again, so a user tab entered during an earlier successful or failed balance becomes the next and
final tab balanced.

The explicit `balance` action keeps its existing contract. It lays the tab out as an even two-axis
grid in reading order, swaps panes when the shape already matches, and otherwise rebuilds through a
scratch tab. It rejects zoomed tabs and invalid or changing layouts, tries to recover staged panes,
and restores its starting pane after focus-changing work or failure. Re-running it on a settled grid
only reapplies target ratios.

Herdr events contain no origin or sequence. The lock orders hook processes, not the focus events that
started them, and a user focus can occur in the narrow interval between the plugin's latest-focus
check and a Herdr swap or move request. That request may itself change focus before the newer hook can
be distinguished; a user choosing exactly the same pane as the expected internal focus is likewise
indistinguishable. These exact races cannot be solved strictly without Herdr metadata. The covered
representative order is a newer user tab focus becoming visible after an earlier balance succeeds or
fails: no old `pane.focus` is issued, the newer tab wins, and that tab is balanced.

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
./scripts/build-plugin.sh
herdr plugin link .
```

After linking the checkout, rebuild and reload the plugin after source changes with:

```sh
mise run reload-plugin
```

The task builds `target/release/herdr-move-pane`, then `scripts/build-plugin.sh` atomically copies it
to `bin/herdr-move-pane`, which is the executable referenced by every manifest action and event.
Reloading Herdr's configuration alone does not update that executable. When no local release binary
exists, the build script instead follows the same verified release-download path used by
installations.

The plugin exposes these actions:

- `jeiea.move-pane.to-next-workspace`
- `jeiea.move-pane.to-previous-workspace`
- `jeiea.move-pane.to-next-tab`
- `jeiea.move-pane.to-previous-tab`
- `jeiea.move-pane.new-tab`
- `jeiea.move-pane.new-workspace`
- `jeiea.move-pane.to-new-tab`
- `jeiea.move-pane.to-new-workspace`
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
