#!/bin/sh
# Build the native port from this checkout (optimized) and run it.
#
#   scripts/play.sh                         boot the whole game (SEGA logo, title, ...)
#   scripts/play.sh --round 1 --substage 2  start play directly in round 1-2
#   scripts/play.sh --help                  every option of the native binary
#
# Arguments are passed to the binary unchanged. The build goes to
# obj/native (CARGO_TARGET_DIR overrides it) and is incremental, so later
# runs only rebuild what changed.
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$project_root/obj/native}"

cargo build --locked --release --manifest-path "$project_root/native/Cargo.toml"
cd "$project_root"
exec "$CARGO_TARGET_DIR/release/ninja-master-native" "$@"
