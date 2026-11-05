#!/bin/sh
# Runs a command against a private tuios daemon, never your own.
#
#   scripts/dev.sh tuios ls
#   scripts/dev.sh target/debug/probe
#
# HOME moves too, because on macOS the tuios config lives under
# ~/Library/Application Support, and that config holds hooks and remote hosts.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
DEV="$ROOT/.dev"
TUIOS_BIN=$(command -v tuios)
mkdir -p "$DEV/run" "$DEV/state" "$DEV/home" "$DEV/config" "$DEV/data"
chmod 700 "$DEV/run"

if [ "${1:-}" = tuios ]; then
  shift
  set -- "$TUIOS_BIN" "$@"
fi

exec env -u TUIOS_SOCKET -u TUIOS_SESSION -u TUIOS_SESSION_ID -u TUIOS_PANE_ID \
  -u TUIOS_WINDOW -u TUIOS_WINDOW_ID -u TUIOS_ENV \
  -u HERDR_SOCKET_PATH -u HERDR_PANE_ID -u HERDR_TAB_ID -u HERDR_WORKSPACE_ID -u HERDR_BIN_PATH \
  HOME="${TESSELLA_DEV_HOME:-$DEV/home}" \
  XDG_RUNTIME_DIR="$DEV/run" \
  XDG_STATE_HOME="$DEV/state" \
  XDG_CONFIG_HOME="$DEV/config" \
  XDG_DATA_HOME="$DEV/data" \
  SHELL=/bin/sh ENV= PS1='$ ' \
  TESSELLA_TUIOS_BIN="$TUIOS_BIN" \
  "$@"
