#!/bin/sh
#MISE description="Run integration tests against Herdr in isolated named sessions"
set -eu

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"

cargo test --locked --test live_herdr_test -- --ignored --test-threads=1
