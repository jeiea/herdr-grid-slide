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

Release maintenance and the checks that remain after making this repository public are documented in
[`docs/releasing.md`](docs/releasing.md).

## License

[MIT](LICENSE)
