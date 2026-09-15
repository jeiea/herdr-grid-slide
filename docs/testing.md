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

The `to-new-tab` scenario invokes the registered action and checks that the detached pane is in
the next tab, tab membership is preserved, and the server focuses the new tab and pane. This is a
headless server smoke test: it verifies that Herdr accepts the requests and preserves final server
state, but it cannot automatically detect a regression where a connected shell client remains on
the source tab.

## Test the minimum supported Herdr version

Minimum support is **0.8.0**. On macOS arm64, with mise, the development tools above, `curl`,
`shasum`, network access, and the local-session permissions above, run:

```sh
mise run test-herdr-0.8.0
```

The [task](../.mise/tasks/test-herdr-0.8.0.sh) downloads the official 0.8.0 binary, verifies its
fixed SHA256, runs the seven isolated scenarios, and removes the temporary download. Other
platforms and checksum mismatches are rejected before execution. No global installation is changed.

## Verify the Herdr 0.9.0 client view

Use a named session with separate configuration, state, and runtime directories; do not use a
normal working session. Connect one shell client, prepare a two-pane source tab, and invoke the
registered `to-new-tab` action from both a middle source tab and the last source tab. For each case,
confirm the new tab position, the detached pane shown and focused in the client, matching server
logs and snapshot focus, and focus stability after the next key input.

For diagnosis, follow [Herdr issue #4153](https://github.com/herdrdev/herdr/issues/4153): on
Herdr 0.9.0, `pane.move --new-tab --focus` can update server focus while the connected client stays
on the source tab. A separate `pane.focus` for the pane ID returned by the move should switch the
client and keep server and screen focus aligned after input. The workaround may also switch other
shell clients in the session because the plugin cannot identify only the invoking client.

Remove the explicit focus only after an upstream #4153 fix is released, the minimum supported Herdr
version excludes affected releases, and both the live smoke and these connected-client checks pass
without the workaround.

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
