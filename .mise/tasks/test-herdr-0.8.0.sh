#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"

platform=$(uname -sm)
case "$platform" in
  'Darwin arm64') ;;
  *) echo "unsupported platform: $platform; requires macOS arm64" >&2; exit 1 ;;
esac

trial_dir=$(mktemp -d "${TMPDIR:-/tmp}/grid-slide-herdr.XXXXXX")
trap 'rm -rf "$trial_dir"' EXIT HUP INT TERM
asset=herdr-macos-aarch64
# Official digest: https://github.com/herdrdev/herdr/releases/tag/v0.8.0
expected=d53a9f93fccfdfcc55632927bf51002f5add0aa7990bcdf508ffbd84ac658178
curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
  "https://github.com/herdrdev/herdr/releases/download/v0.8.0/$asset" \
  --output "$trial_dir/$asset"
actual=$(shasum -a 256 "$trial_dir/$asset" | awk '{ print $1 }')
if [ "$actual" != "$expected" ]; then
  echo "checksum mismatch for $asset" >&2
  exit 1
fi
chmod +x "$trial_dir/$asset"
ln -s "$trial_dir/$asset" "$trial_dir/herdr"
export PATH="$trial_dir:$PATH"
herdr --version
HERDR_TEST_VERSION='herdr 0.8.0' mise run live-herdr
