# herdr-move-pane

Move the current Herdr pane cyclically to the adjacent tab or workspace, create a tab or workspace
immediately after the current one, move the pane to a new tab or workspace, reorder workspaces,
navigate or move panes directionally in visual order across container boundaries, split a pane, or
lay a tab out as an even grid. Directional moves swap panes within a tab and move them across tabs or
workspaces at an edge. Scoped pane moves split an existing destination on the right at a 50:50 ratio
and keep focus. Move, split, and new pane actions do not balance layouts themselves, except that a
successful tab-scoped move or horizontal directional boundary move starts automatic balance before
returning. It balances the destination if that tab is still focused after the balance lock is
acquired.

Workspace-scoped pane moves target the active tab of the adjacent workspace and use visible
workspace `number` order. Tab-scoped pane moves stay in the current workspace and follow displayed
tab order. Both wrap at either end. `move-workspace` instead moves the active workspace itself
through workspace `number` order. Directional focus preserves its cross-axis position where possible
and wraps across tabs or workspaces at an edge.

Directional actions treat tabs as connected from left to right and workspaces as connected from top
to bottom. `move-left`, `move-right`, `move-up`, and `move-down` select the same geometric target as
their `focus-*` counterpart. A target in the same tab swaps with the current pane. At a boundary,
left and right move into the previous or next tab, while up and down move into the active tab of the
previous or next workspace. A pane enters on the movement edge: right of the target when moving left,
left when moving right, below when moving up, and above when moving down. The cross-axis position is
preserved on a best-effort basis. After a move or swap completes, focus follows the moved pane.

Herdr only inserts on the right or below a target. A boundary `move-right` or `move-down` therefore
moves without focus and then swaps the moved pane with the target. If that swap request fails, its
response is invalid, or Herdr declines it, the pane remains moved and the action reports the partial
completion.

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

`to-new-workspace` moves the focused tab into a focused new workspace and places that workspace
immediately after its source in visible `number` order. Herdr has no request that carries a tab
across workspaces, so the first pane in reading order opens the new workspace and the remaining
panes follow it in that order, after which focus returns to the pane that had it and the new tab is
balanced like the destination of a tab-scoped move, keeping its pane order. The whole move holds the
balance lock, so the `pane.focused` hooks it fires wait for it and then find nothing left to do. A
custom tab label carries over, except one that spells the tab's own position number: Herdr 0.8.2's
snapshot cannot tell it from the default label, and carrying it over would pin the new tab to a
stale position. This comparison can go once Herdr reports whether a label is custom. If the source
was already the last workspace, the new workspace is already in the right place and no reorder
request is needed. Moving a workspace's only tab would close that workspace, so the action ignores
that case without changing the session. If Herdr declines the first pane move, the action likewise
stops without further requests.

`split-pane` and `new-pane` are aliases. They split the focused pane at 50:50 along the direction
the tab already grows in. For mixed or single-pane layouts, a pane wider than twice its height splits
right; all others split down.

`pane.focused`, successful tab-scoped moves, successful multi-pane `to-new-workspace` moves, and
successful horizontal directional boundary moves enter automatic balance. The plugin records the
latest entered tab and its pane set in `HERDR_PLUGIN_STATE_DIR/balance-focus.json`; another pane
focus in that same tab with the same panes does nothing, while a focus that arrives with a new or
closed pane -- such as the one `new-pane` creates -- balances the tab again. A tab-scoped move
ignores a matching cached pane set once when the first snapshot after locking still focuses its
expected destination. If that snapshot focuses another tab, the expected destination is discarded
and the usual latest focus and cache rules apply. This makes an externally moved pane balance its
destination when entered and leaves the source to be balanced when the user later enters it. Hook
processes share a file lock, read the latest session snapshot after acquiring it, and keep following
newer tab focus until the observed tab is settled. Internal focus events from swaps, scratch-tab
moves, and rebuilding therefore collapse into the same run instead of starting recursive work. With
no state yet, the first observed focus is treated as a tab entry because Herdr does not provide the
previous tab.

If a tab-scoped or horizontal directional boundary move succeeds but automatic balance fails, the
pane remains moved and the action fails with `pane moved, but automatic balance failed: ...`.

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

If `to-new-workspace` moves the first pane but Herdr omits the created tab ID, or a following pane
cannot be moved, the action fails with an error that names the panes still in the source tab. The
panes already moved remain in the new workspace at the end of the session. If every pane moved but
restoring focus fails, Herdr omits the created workspace ID, or the subsequent workspace reorder
fails, the action fails with an error that says the tab move already completed; that workspace may
remain at the end of the session. If the tab moved but automatic balance fails, the action fails
with `tab moved, but automatic balance failed: ...`.

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

Requires mise and Herdr 0.8 or later. Mise supplies the pinned Rust 1.97.1 and ShellCheck 0.9.0
toolchains.

Build and apply the local plugin for both the first link and later source changes with:

```sh
mise run apply-local
```

The task builds `target/release/herdr-move-pane` with the locked dependencies, atomically copies it
to `bin/herdr-move-pane`, and links the checkout. An existing `bin` copy remains available if the
build or replacement fails and is independent of Cargo's disposable `target` directory.

Run the local quality checks with:

```sh
mise run check
```

The default checks keep the fast, deterministic `FakeHerdr` integration suite for socket requests,
branch coverage, errors, and races. To also smoke-test representative directional moves against an
isolated Herdr 0.8.2 named session, run:

```sh
mise run live-herdr
```

The live test links a temporary copy of the manifest and Cargo-built binary, invokes the real plugin
actions, and removes its named sessions and temporary Herdr paths afterward. It does not modify the
repository `bin/` directory or user plugin links.

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
- `jeiea.move-pane.move-left`
- `jeiea.move-pane.move-down`
- `jeiea.move-pane.move-up`
- `jeiea.move-pane.move-right`
- `jeiea.move-pane.split-pane`
- `jeiea.move-pane.new-pane`
- `jeiea.move-pane.balance`

Release maintenance and the checks that remain after making this repository public are documented in
[`docs/releasing.md`](docs/releasing.md).

## License

[MIT](LICENSE)
