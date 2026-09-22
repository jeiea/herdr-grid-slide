#!/bin/sh
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"

shellcheck_version=$(shellcheck --version | awk '$1 == "version:" { print $2 }')
if [ "$shellcheck_version" != 0.11.0 ]; then
  echo "ShellCheck 0.11.0 is required, found ${shellcheck_version:-unknown}" >&2
  exit 1
fi

./scripts/check-version.sh
shellcheck scripts/*.sh tests/*.sh .mise/tasks/*
for test_script in tests/*.sh
do
  sh "$test_script"
done
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release
