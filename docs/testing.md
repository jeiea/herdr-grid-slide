# Verification

## Run the local quality gate

From a repository checkout on macOS or Linux, install the configured tools with mise and run:

```sh
mise install
mise run check
```

Checks versions, shell scripts and their tests, Rust formatting and Clippy, unit and
socket-fixture integration tests, and a release build. Live Herdr scenarios run separately.

## Check Windows support

On Windows x64, with Rust 1.97.1, Windows PowerShell 5.1 and `curl.exe`, run:

```powershell
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

The `Windows x64 integration` CI job runs these commands on `windows-2025`. All 111 shared
integration tests use real named pipes on Windows and Unix sockets on macOS/Linux. Two additional
Windows integration tests cover waiting for a busy Herdr connection and installing a release.
The latter invokes the manifest's PowerShell build command against a local release containing
the compiled `.exe`, then runs a registered action using Herdr's absolute path resolution with
the extension omitted. It also checks replacement, corrupt downloads, duplicate/invalid/missing
checksums, unsupported architecture, curl failure propagation, preservation of the installed
binary, and temporary-file cleanup. These tests use a fake Herdr server at the OS transport boundary.

From a non-Windows checkout, check the Windows Rust code without running it:

```sh
rustup target add x86_64-pc-windows-msvc
cargo check --locked --target x86_64-pc-windows-msvc --all-targets
cargo clippy --locked --target x86_64-pc-windows-msvc --all-targets -- -D warnings
```

Cross-checking does not execute named pipes, Windows file locks, PowerShell, or `.exe` resolution.
The live Herdr scenarios and `apply-local` task still require a Unix environment. Confirm actual
Windows CI results to complete the current verification. Checking registered actions and automatic
balancing in Windows Herdr is separate, optional follow-up verification.

## Run the live scenarios

With Herdr 0.8.2 on PATH, run the command below. If previously set, first unset `HERDR_TEST_VERSION`.

```sh
mise run live-herdr
```

The harness defaults to the exact output `herdr 0.8.2`; `HERDR_TEST_VERSION` can override the
expected output. It runs eight scenarios in isolated named sessions and cleans up its servers
and session data. The environment must permit local Herdr servers, Unix sockets, and terminal processes.

To exercise an installed Herdr 0.9.0:

```sh
HERDR_TEST_VERSION='herdr 0.9.0' mise run live-herdr
```

The scenarios invoke `move-left`, `move-right`, `move-up`, `move-down`, `to-new-tab`,
`to-new-workspace`, `tab-to-new-workspace`, `tab-to-next-workspace`, and
`tab-to-previous-workspace`. They cover a horizontal tab-boundary round trip, vertical workspace
moves, separate fixtures that distinguish a one-pane move from a whole-tab move, and focused or
non-focused pane termination through the API or process exit. The non-leading `tab-to-new-workspace`
case also guards the old pane-ID alias used to refocus a follower after its tab moves. The suite
still has eight isolated scenarios because both new-workspace actions share one scenario. This is
a headless server smoke test: it verifies that Herdr accepts the requests and preserves final
server state, but it cannot observe which tab each connected shell client displays.

## Test the minimum supported Herdr version

Minimum support is **0.8.0**. On macOS arm64, with mise, the development tools above, `curl`,
`shasum`, network access, and the local-session permissions above, run:

```sh
mise run test-herdr-0.8.0
```

The [task](../.mise/tasks/test-herdr-0.8.0.sh) downloads the official 0.8.0 binary, verifies its
fixed SHA256, runs the eight isolated scenarios, and removes the temporary download. Other
platforms and checksum mismatches are rejected before execution. No global installation is changed.

## Verify Herdr 0.9.0 connected-client views

Use a named session with separate configuration, state, and runtime directories; do not use a
normal working session. Link a temporary copy of the plugin into that session and record the exact
key bindings used. With one connected shell client and the smallest practical fixture:

1. Use the actual left and right directional keys to cross a tab boundary in both directions.
2. Invoke direct next-tab and next-workspace pane moves.
3. Invoke `to-new-workspace` in a multi-pane tab. Confirm that only the focused pane moves,
   the source tab remains, the new workspace appears immediately after the source, and the moved
   pane receives the next input.
4. Move a separate whole multi-pane tab to a new workspace while its leading pane is focused.
   Confirm that all panes and the tab label move together and the focused pane receives the next input.
5. Invoke `to-new-tab`, then use the normal new-tab and new-workspace keys as negative cases.

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

On 2026-09-22, macOS arm64, for the Windows support change:

| Check | Result |
| --- | --- |
| `mise run check` | 4 shell test scripts, 3 Rust unit tests, 111 integration tests, formatting, Clippy and release build passed; 8 live scenarios remained ignored |
| Windows x64 `cargo check --locked --all-targets` and Clippy with warnings denied | Passed with `--target x86_64-pc-windows-msvc`, including compilation of the two Windows-only integration tests |
| Actionlint and native no-argument release smoke | Passed; the native binary exited with code 1 and printed `usage:` |
| Version-input regression | Changing the PowerShell installer after a release tag required a version bump |
| macOS PowerShell 7.5.6 installer check, reported by the coordinator | A copy of the final `build-plugin.ps1`, with `curl.exe` mapped to `/usr/bin/curl` and a local `file://` release, exited with code 0 for both initial installation and reinstallation; this does not verify Windows PowerShell 5.1 |
| Windows runtime, PowerShell 5.1 installer, remote CI and release | Not run: no Windows environment or access to the remote repository was available |

The Windows integration suite has 113 cases (111 shared plus 2 Windows-only cases). Cross-target
compilation is not evidence that those cases pass on Windows. No release was published.

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

In the pane-termination verification on 2026-09-15, macOS 26.6.2 arm64:

| Check | Result |
| --- | --- |
| Targeted termination socket integration tests | A `pane.closed` hook without `HERDR_TAB_ID` balanced the focused tab and updated its state; explicit `balance` without that variable still failed before making a request; all three manifest events used an accepted entrypoint |
| `mise run check` | 4 shell test scripts, 3 Rust unit tests, and 104 integration tests passed; release build succeeded |
| `mise run test-herdr-0.8.0` | Official download and fixed SHA256 check passed; `herdr 0.8.0`; all 8 isolated live scenarios passed |
| `mise run live-herdr` with Herdr 0.8.2 on PATH | `herdr 0.8.2`; all 8 isolated live scenarios passed |
| `HERDR_TEST_VERSION='herdr 0.9.0' mise run live-herdr` | `herdr 0.9.0`; all 8 isolated live scenarios passed |

The termination scenario covers focused and non-focused API closure plus focused process exit. Each
segment verifies a finite successful termination hook, the surviving pane and terminal set, a
horizontal 50:50 grid, focus on a survivor, and successful input to that focused pane. The harness
removes inherited `HERDR_*` values and uses short isolated session names; all temporary session and
download resources were removed afterward.

On 2026-09-16, macOS 26.6.2 arm64:

| Check | Result |
| --- | --- |
| Targeted pane/new-workspace integration tests | 7 passed, including no state directory or copied label, no-op and refusal paths, focus and placement partial failures, sibling-tab allowance, and last-workspace placement |
| Existing whole-tab and manifest regression tests | Leading-pane whole-tab move, custom label retention, and every manifest entrypoint passed |
| `mise run check` | Stopped before repository checks because mise's ShellCheck 0.9.0 executable was x86_64 and macOS reported `Bad CPU type in executable` |
| Quality-gate steps other than ShellCheck | Version check, 4 shell test scripts, Rust formatting, Clippy with warnings denied, 3 unit tests, 111 integration tests, and release build passed |
| `mise run test-herdr-0.8.0` | Official download and fixed SHA256 check passed; `herdr 0.8.0`; all 8 isolated live scenarios passed |
| `mise run live-herdr` with a fixed Herdr 0.8.2 path | `herdr 0.8.2`; all 8 isolated live scenarios passed |
| `HERDR_TEST_VERSION='herdr 0.9.0' mise run live-herdr` | Installed `herdr 0.9.0`; all 8 isolated live scenarios passed |

The expanded new-workspace scenario separates one focused pane first, verifies its source tab,
placement, focus, and next input, then moves an independent three-pane labeled tab and verifies its
order, label, focus, and next input. ShellCheck remains unverified in this environment; no user tool
installation or mise setting was changed. The Herdr 0.9.0 connected-client procedure was not run
in this round.
