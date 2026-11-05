#!/bin/sh
# Tries Tessella on a private tuios daemon, so your live sessions are not touched.
# Keys and hosts come from a copy of your tuios config, without its hooks. New panes
# on another machine are real shells on that machine.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
mkdir -p .dev/config/tuios
python3 - <<'PY'
import os, re
home = os.path.expanduser("~")
for source in (home + "/Library/Application Support/tuios/config.toml", home + "/.config/tuios/config.toml"):
    if os.path.exists(source):
        text = open(source).read()
        # Hooks run commands on this machine, such as notifications. They stay out of a trial.
        open(".dev/config/tuios/config.toml", "w").write(re.sub(r"(?ms)^\[hooks\].*?(?=^\[|\Z)", "", text))
        break
PY
[ -x target/release/tessella ] || cargo build --release --bin tessella
scripts/dev.sh target/release/tessella
