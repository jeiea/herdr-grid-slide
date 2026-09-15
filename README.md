# Grid Slide

A plugin born from the idea of focusing and moving panes with hjkl.

## Support

Minimum Herdr version: **0.8.0**. Release targets: macOS and Linux, each on arm64 and x86_64.
The eight live scenarios passed on macOS arm64 with Herdr 0.8.0, 0.8.2, and 0.9.0; other
platforms and versions have not been exercised in that run. See
[verification details](docs/testing.md#recorded-results).

## Install

Installation requires a published release.

```sh
herdr plugin install jeiea/herdr-grid-slide
```

Requires Herdr, git, and the installer tools: `sh`, `curl`, `awk`, basic Unix utilities,
and either `sha256sum` or `shasum`. Release binary installation does not require Rust.

## Behavior

`to-new-workspace` moves the current **whole tab** immediately after its source workspace;
it does nothing when that tab is the workspace's only tab. Previous/next workspace tab actions
append an independent tab at the destination, closing the source workspace if it becomes empty.

Successful pane moves across tab or workspace boundaries keep the server focus and connected
shell view on the moved pane. See [movement and balancing](docs/behavior.md) for compensation
timing, partial-success behavior, and the current multi-client limitation.

Tabs are automatically balanced when a focus or pane termination event observes a tab entry,
changed panes, or changed area, and on certain move paths. Balancing can change split structure
and proportions; a temporary tab may appear.

## Checks

With mise and the configured Rust and ShellCheck installed, run `mise run check` for the local
quality gate. On macOS arm64, `mise run test-herdr-0.8.0` downloads and tests the minimum version.
With Herdr 0.8.2 on PATH, `mise run live-herdr` runs eight isolated session scenarios. Set
`HERDR_TEST_VERSION='herdr 0.9.0'` to check an installed Herdr 0.9.0 instead.
See [verification prerequisites](docs/testing.md#test-the-minimum-supported-herdr-version)
and the [release procedure](docs/releasing.md).

## Configuration

`~/.config/herdr/config.toml`

```toml
[[keys.command]]
key = "alt+h"
type = "plugin_action"
command = "jeiea.grid-slide.focus-left"
description = "Focus left or wrap to previous tab"

[[keys.command]]
key = "alt+j"
type = "plugin_action"
command = "jeiea.grid-slide.focus-down"
description = "Focus down or wrap to next workspace"

[[keys.command]]
key = "alt+k"
type = "plugin_action"
command = "jeiea.grid-slide.focus-up"
description = "Focus up or wrap to previous workspace"

[[keys.command]]
key = "alt+l"
type = "plugin_action"
command = "jeiea.grid-slide.focus-right"
description = "Focus right or wrap to next tab"

[[keys.command]]
key = "alt+shift+h"
type = "plugin_action"
command = "jeiea.grid-slide.move-left"
description = "Move pane left"

[[keys.command]]
key = "alt+shift+j"
type = "plugin_action"
command = "jeiea.grid-slide.move-down"
description = "Move pane down"

[[keys.command]]
key = "alt+shift+k"
type = "plugin_action"
command = "jeiea.grid-slide.move-up"
description = "Move pane up"

[[keys.command]]
key = "alt+shift+l"
type = "plugin_action"
command = "jeiea.grid-slide.move-right"
description = "Move pane right"

[[keys.command]]
key = "ctrl+alt+k"
type = "plugin_action"
command = "jeiea.grid-slide.tab-to-previous-workspace"
description = "Move tab to previous workspace"

[[keys.command]]
key = "ctrl+alt+j"
type = "plugin_action"
command = "jeiea.grid-slide.tab-to-next-workspace"
description = "Move tab to next workspace"

[[keys.command]]
key = "alt+n"
type = "plugin_action"
command = "jeiea.grid-slide.new-pane"
description = "New pane"

[[keys.command]]
key = "alt+shift+n"
type = "plugin_action"
command = "jeiea.grid-slide.new-tab"
description = "Create tab to the right"

[[keys.command]]
key = "ctrl+alt+n"
type = "plugin_action"
command = "jeiea.grid-slide.new-workspace"
description = "Create workspace after current"

[[keys.command]]
key = "alt+m"
type = "plugin_action"
command = "jeiea.grid-slide.to-new-tab"
description = "Move pane to new tab"

[[keys.command]]
key = "ctrl+alt+m"
type = "plugin_action"
command = "jeiea.grid-slide.to-new-workspace"
description = "Move tab to new workspace"

[[keys.command]]
key = "alt+shift+i"
type = "plugin_action"
command = "jeiea.grid-slide.move-workspace-previous"
description = "Move workspace previous"

[[keys.command]]
key = "alt+shift+o"
type = "plugin_action"
command = "jeiea.grid-slide.move-workspace-next"
description = "Move workspace next"

[[keys.command]]
key = "alt+."
type = "plugin_action"
command = "jeiea.grid-slide.balance"
description = "Balance in tab"
```
