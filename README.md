# Grid Slide

A plugin born from the idea of focusing and moving panes with hjkl.

## Install

```sh
herdr plugin install jeiea/herdr-grid-slide
```

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
description = "Move pane to new workspace"

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
