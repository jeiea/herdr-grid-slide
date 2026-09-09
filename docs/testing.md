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
expected output. It runs six scenarios in isolated named sessions and cleans up its servers
and session data. The environment must permit local Herdr servers, Unix sockets, and terminal processes.

## Test the minimum supported Herdr version

Minimum support is **0.8.0**. On macOS arm64, with mise, the development tools above, `curl`,
`shasum`, network access, and the local-session permissions above, run:

```sh
mise run test-herdr-0.8.0
```

The [task](../.mise/tasks/test-herdr-0.8.0.sh) downloads the official 0.8.0 binary, verifies its
fixed SHA256, runs the six isolated scenarios, and removes the temporary download. Other
platforms and checksum mismatches are rejected before execution. No global installation is changed.

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
