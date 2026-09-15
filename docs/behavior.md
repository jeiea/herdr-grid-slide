# Movement and balancing

## Whole-tab workspace moves

| Action | Destination | No-op condition |
| --- | --- | --- |
| `to-new-workspace` | New workspace immediately after the source | Source has only one tab |
| `tab-to-previous-workspace` | Independent tab at the previous workspace's end | Only one workspace exists |
| `tab-to-next-workspace` | Independent tab at the next workspace's end | Only one workspace exists |

Previous/next follows visible workspace order and wraps at either end. When moving the source's
last tab to an existing workspace, moving its final pane closes the empty source workspace.
The moves carry all panes, retain the focused terminal, and arrange the destination in reading
order. Tab labels are retained unless they exactly match the source tab's one-based position
number (for example, `2` on the second tab), which is treated as a default label even if manually
assigned. The moves do not preserve the original split tree or ratios.

Illustrative whole-tab move (`*` marks the focused terminal, brackets mark tabs):

```text
Before: W1 [keep] [A B* C]  | W2 [other]
Action: to-new-workspace
After:  W1 [keep] | new W [A B* C] | W2 [other]
```

## Cross-container focus recovery

After a successful public pane move crosses a tab or workspace boundary, both Herdr's server focus
and connected shell client view point to the moved pane in its destination. The plugin explicitly
focuses that pane because affected Herdr releases, including 0.9.0, can update server focus for
`pane.move` without projecting the move to connected client views.

The compensation runs once at the final structural boundary of each logical move:

- Direct previous/next tab or workspace moves focus the pane returned by `pane.move`, before any
  destination balancing.
- Directional moves focus the returned pane after anchor state is recorded and, for a rightward
  boundary move, after the required swap, but before automatic balancing.
- Whole-tab workspace moves focus after every following pane has joined the destination. A focused
  leading pane uses the new ID from the leading move response; a focused later pane uses the old ID
  alias that supported Herdr versions resolve to its moved pane.
- `to-new-tab` focuses the detached pane before positioning its new tab.

If this focus request fails, the structural move remains complete but later balancing or new
tab/workspace positioning does not start. A failed or refused rightward follow-up swap is earlier:
the pane remains in the destination tab, the connected view remains on the source tab, no focus
compensation is sent, and no new anchor is recorded. Same-tab swaps, splits, pane creation, and
internal `focus: false` balancing moves do not receive this compensation.

The public `pane.focus` request projects to every connected shell client in the same server. A move
started from one shell can therefore switch other shells to the destination tab because the plugin
does not receive an invoking-client identifier. This limitation remains preferable to leaving the
invoking view behind. The workaround for `pane.move --focus` may be removed only after an upstream
[#4153](https://github.com/herdrdev/herdr/issues/4153) fix is in every supported release and the
connected-view checks pass without it. Rightward compensation additionally requires evidence that
`pane.move(focus: false)` followed by `pane.swap` projects the client view by itself.

## Automatic balancing

The `pane.focused` hook observes the focused tab, its pane set, and its occupied area. A tab entry
or a change to the panes or area triggers balancing. Repeated focus within the same observed
state skips duplicate work. A resize or client attach alone does not run a separate resize hook;
a subsequent pane focus lets the plugin observe the new area.

Pane moves between tabs (`to-next-tab`, `to-previous-tab`, and horizontal directional moves
that cross a tab boundary) directly request destination balancing. Whole-tab workspace moves
with multiple panes also directly balance the destination. Vertical directional moves and
`to-next-workspace` / `to-previous-workspace` rely on the resulting focus hook for automatic balancing.
Across vertical workspace boundaries, the pane enters after the selected boundary pane in reading
order. For example, the focus hook places a right pane returned by `move-up` then `move-down` after the
original left pane; zoomed tabs skip that balancing and can retain Herdr's raw below-target placement.

Automatic balancing skips tabs with one pane and zoomed tabs. Explicit `balance` also leaves
a one-pane tab unchanged, but reports an error for a zoomed tab until it is unzoomed.

## Layout changes

Balancing reads panes from top to bottom, then left to right, and lays them out in an even grid
suited to the available area. Existing split structure and proportions can change. Rebuilding
the grid may briefly create a temporary tab while moving panes out and back.

Illustrative four-pane result in an area suited to two rows and two columns:

```text
Before: A | B | C | D
After:  A | B
        C | D
```

The grid shape depends on pane count and area.
See [the recorded verification scope](testing.md#recorded-results).
