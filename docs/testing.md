# Verification

## Run the local quality gate

From a repository checkout, install the configured tools with mise and run:

```sh
mise install
mise run check
```

Checks versions, shell scripts and their tests, Rust formatting and Clippy, unit and
socket-fixture integration tests, and a release build. Live Herdr scenarios run separately.

## Run the live scenarios

With Herdr 0.8.2 on PATH, run the command below. If previously set, first unset `HERDR_TEST_VERSION`.

```sh
mise run live-herdr
```

The harness defaults to the exact output `herdr 0.8.2`; `HERDR_TEST_VERSION` can override the
expected output. It runs seven scenarios in isolated named sessions and cleans up its servers
and session data. The environment must permit local Herdr servers, Unix sockets, and terminal processes.

To exercise an installed Herdr 0.9.0:

```sh
HERDR_TEST_VERSION='herdr 0.9.0' mise run live-herdr
```

The scenarios invoke `move-left`, `move-right`, `move-up`, `move-down`, `to-new-tab`,
`to-new-workspace`, `tab-to-next-workspace`, and `tab-to-previous-workspace`. They cover a
horizontal tab-boundary round trip, vertical workspace moves, and whole-tab moves. The non-leading
`to-new-workspace` case also guards the old pane-ID alias used to refocus a follower after its tab
moves. This is a headless server smoke test: it verifies that Herdr accepts the requests and
preserves final server state, but it cannot observe which tab each connected shell client displays.

## Test the minimum supported Herdr version

Minimum support is **0.8.0**. On macOS arm64, with mise, the development tools above, `curl`,
`shasum`, network access, and the local-session permissions above, run:

```sh
mise run test-herdr-0.8.0
```

The [task](../.mise/tasks/test-herdr-0.8.0.sh) downloads the official 0.8.0 binary, verifies its
fixed SHA256, runs the seven isolated scenarios, and removes the temporary download. Other
platforms and checksum mismatches are rejected before execution. No global installation is changed.

## Verify Herdr 0.9.0 connected-client views

Use a named session with separate configuration, state, and runtime directories; do not use a
normal working session. Link a temporary copy of the plugin into that session and record the exact
key bindings used. With one connected shell client and the smallest practical fixture:

1. Use the actual left and right directional keys to cross a tab boundary in both directions.
2. Invoke direct next-tab and next-workspace pane moves.
3. Move a whole multi-pane tab to a new workspace while its leading pane is focused.
4. Invoke `to-new-tab`, then use the normal new-tab and new-workspace keys as negative cases.

For every move, compare the displayed workspace/tab header and pane body with the server snapshot,
confirm the action and any `pane.focused` hooks finish without repetition, then enter another shell
command to verify that input still reaches the displayed pane. Repeat one representative move with
two connected shell clients and confirm that both views switch to the destination. Compare that
result with the [documented multi-client limitation](behavior.md#cross-container-focus-recovery).

For diagnosis, follow [Herdr issue #4153](https://github.com/herdrdev/herdr/issues/4153): on
Herdr 0.9.0, `pane.move --new-tab --focus` can update server focus while the connected client stays
on the source tab. A separate `pane.focus` for the pane ID returned by the move should switch the
client and keep server and screen focus aligned after input.

See [cross-container focus recovery](behavior.md#cross-container-focus-recovery) for the workaround
rationale, multi-client constraint, and removal gates.

## Recorded results

On 2026-09-09, macOS 26.6.2 arm64:

| Check | Result |
| --- | --- |
| `mise run test-herdr-0.8.0` | Official download and fixed SHA256 check passed; `herdr 0.8.0`; 6 scenarios passed; download and session directories cleaned |
| `mise run check` with the new task | 4 shell test scripts, 3 Rust unit tests, 98 integration tests passed; release build succeeded |
| Earlier Herdr 0.8.0 / 0.8.2 live runs | 6 scenarios passed on each version |
| Earlier expected 0.8.0 / actual 0.8.2 version guard | Rejected before server startup |

The scenarios cover pane and whole-tab moves through registered actions, including focus,
reading order, destination independence, source workspace removal, and balancing assertions.
This evidence is limited to the tested versions, scenarios, and platform. Public installation,
actual draft-release access, other platforms, and all-action coverage remain unverified.
See the [release checklist](releasing.md).

On 2026-09-15, macOS 26.6.2 arm64:

| Check | Result |
| --- | --- |
| `mise run check` | 4 shell test scripts, 3 Rust unit tests, and 99 integration tests passed; release build succeeded |
| `mise run test-herdr-0.8.0` | Official download and fixed SHA256 check passed; `herdr 0.8.0`; 7 isolated live scenarios passed |
| `mise run live-herdr` with Herdr 0.8.2 on PATH | 7 isolated live scenarios passed |
| `HERDR_TEST_VERSION='herdr 0.9.0' mise run live-herdr` | 7 isolated live scenarios passed |
| Herdr 0.9.0 connected-client workaround probe | `pane.move --new-tab --focus` left the client on the source tab while server focus moved; direct `pane.focus` switched the client and focus remained after input |
| Herdr 0.9.0 registered `to-new-tab` acceptance | Middle and last source-tab cases passed in separate isolated connected clients; new-tab position, detached-pane screen and focus, request logs, snapshots, and focus after input matched expectations |

The 0.8.0 result keeps `min_herdr_version = "0.8.0"`. The connected-client checks cover the
0.9.0 view behavior that the seven headless scenarios cannot observe.

Later on 2026-09-15, macOS 26.6.2 arm64:

| Check | Result |
| --- | --- |
| `mise run check` | 4 shell test scripts, 3 Rust unit tests, and 101 integration tests passed; release build succeeded |
| `mise run test-herdr-0.8.0` | Official download and fixed SHA256 check passed; `herdr 0.8.0`; all 7 isolated live scenarios passed |
| `mise run live-herdr` with Herdr 0.8.2 on PATH | `herdr 0.8.2`; all 7 isolated live scenarios passed |
| `HERDR_TEST_VERSION='herdr 0.9.0' mise run live-herdr` | `herdr 0.9.0`; all 7 isolated live scenarios passed |
| Non-leading whole-tab alias guard | On 0.8.0, 0.8.2, and 0.9.0, the moved follower changed from `w1:p4` to `w3:p3`, and focusing its old `w1:p4` alias selected `w3:p3` |
| Herdr 0.9.0 connected-client acceptance | Actual left/right keys, direct next-tab/next-workspace moves, a leading-pane whole-tab move, `to-new-tab`, and unchanged new-tab/new-workspace creation all matched the client view, server focus, completed logs, and next input |
| Herdr 0.9.0 two-client acceptance | Both connected shells switched on the representative cross-tab move; the action and finite focus hooks completed without repetition |

The directional live scenario now covers a right-then-left round trip without increasing the seven
scenario count. The isolated connected-client fixture was removed after both clients detached and
the server stopped. The headless suite still cannot observe individual client views. The two-client
result demonstrates the [documented multi-client limitation](behavior.md#cross-container-focus-recovery).

In the final 2026-09-15 vertical-side verification on macOS 26.6.2 arm64:

| Check | Result |
| --- | --- |
| Targeted vertical socket integration tests | Left anchors for `Up` and `Down` used the new pane ID in `focus: false` move, swap, and focus order; equality kept `focus: true` without a swap; a failed vertical follow-up swap preserved the previous anchor and stopped before focus or balancing |
| `mise run check` | 4 shell test scripts, 3 Rust unit tests, and 102 integration tests passed; release build succeeded |
| `mise run test-herdr-0.8.0` | Official download and fixed SHA256 check passed; `herdr 0.8.0`; all 7 isolated live scenarios passed |
| `mise run live-herdr` with Herdr 0.8.2 on PATH | `herdr 0.8.2`; all 7 isolated live scenarios passed |
| `HERDR_TEST_VERSION='herdr 0.9.0' mise run live-herdr` | `herdr 0.9.0`; all 7 isolated live scenarios passed |

The expanded vertical scenario keeps a right pane on the right through a pure round trip, keeps a
left pane on the left through a pure round trip, and preserves a user-selected left side before
both `Down` and the symmetric `Up`. Every vertical segment verifies the destination workspace and
tab, moving terminal, server focus, reading order, unaffected panes, and finite successful
`pane.focused` hooks. Each live run used isolated session resources that were removed afterward.
