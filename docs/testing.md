# Verification

## Run the local quality gate

From a repository checkout, with mise installed, install the tools in `mise.toml` and run:

```sh
mise install
mise run check
```

The gate checks versions, shell scripts and their tests, Rust formatting and Clippy, Rust unit
and socket-fixture integration tests, and a release build. It does not run the ignored live Herdr
scenarios or exercise public installation.

## Run the live scenarios

With Herdr 0.8.2 on PATH:

```sh
mise x -- herdr --version
mise run live-herdr
```

The harness requires the exact output `herdr 0.8.2` by default. `HERDR_TEST_VERSION` overrides
the expected output, including the `herdr ` prefix; it does not select or download an executable.
The existing PATH lookup selects `herdr`, and the exact version comparison remains mandatory.

The harness copies the built plugin into temporary directories, links it only in isolated
named sessions, invokes registered actions, and cleans up its servers and session data.
It does not install the plugin into the default session. The environment must permit local
Herdr servers, Unix sockets, and terminal processes.

## Test the minimum supported Herdr version

The minimum remains **0.8.0**. This procedure is for macOS arm64, requires `gh` with API access,
`curl`, `shasum`, and the development tools above, and runs from the repository root.
It obtains the official [v0.8.0 release asset](https://github.com/herdrdev/herdr/releases/tag/v0.8.0),
checks its published SHA256 digest before execution, and selects it through a temporary PATH.

```sh
mise x -- sh <<'SH'
set -eu
trial_dir=$(mktemp -d "${TMPDIR:-/tmp}/grid-slide-herdr.XXXXXX")
trap 'rm -rf "$trial_dir"' EXIT HUP INT TERM
asset=herdr-macos-aarch64
digest=$(gh api repos/herdrdev/herdr/releases/tags/v0.8.0 \
  --jq '.assets[] | select(.name == "herdr-macos-aarch64") | .digest')
test "$digest" = sha256:d53a9f93fccfdfcc55632927bf51002f5add0aa7990bcdf508ffbd84ac658178
curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
  "https://github.com/herdrdev/herdr/releases/download/v0.8.0/$asset" \
  --output "$trial_dir/$asset"
actual=$(shasum -a 256 "$trial_dir/$asset" | awk '{ print $1 }')
test "sha256:$actual" = "$digest"
chmod +x "$trial_dir/$asset"
ln -s "$trial_dir/$asset" "$trial_dir/herdr"
export PATH="$trial_dir:$PATH"
command -v herdr
herdr --version
HERDR_TEST_VERSION='herdr 0.8.0' mise run live-herdr
SH
```

For the 0.8.2 regression run, use its existing installation on PATH and the default expectation:

```sh
mise x -- sh -eu -c 'unset HERDR_TEST_VERSION; herdr --version; mise run live-herdr'
```

## Recorded results

On 2026-09-09, macOS 26.6.2 arm64:

| Check | Result |
| --- | --- |
| Official Herdr 0.8.0 macOS arm64 binary | SHA256 matched the digest above; output `herdr 0.8.0` |
| Live scenarios with Herdr 0.8.0 via temporary PATH | 6 passed |
| Live scenarios with Herdr 0.8.2 via existing PATH | 6 passed |
| Exact version guard with expected 0.8.0 and actual 0.8.2 | Rejected before server startup |
| `mise run check` | 4 shell test scripts, 3 Rust unit tests, 98 integration tests passed; release build succeeded |

The minimum-version shell block above was executed unchanged and exited with status 0:
official asset download, published digest comparison, temporary PATH selection of Herdr 0.8.0,
and all six live scenarios passed.

The six scenarios exercise plugin linking and registered action calls: `move-right` within a
tab and across a tab boundary, `move-down` into the next workspace's active tab,
`to-new-workspace` carrying a whole tab, previous/next whole-tab round trips, and moving a
workspace's only tab with one or two panes. Existing assertions cover terminal identity,
focus, reading order, custom labels, destination independence and position, source removal,
the two-pane equal split, and no-ops with one remaining workspace where applicable.

This is evidence for those scenarios on those two versions and that platform, not every action,
every later version, or all four release targets. Public installation and actual draft-release
asset access remain unverified. See the [release checklist](releasing.md).
