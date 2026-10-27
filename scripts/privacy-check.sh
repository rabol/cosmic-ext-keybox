#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Privacy check (specification §8, acceptance 10).
#
# Runs a debug build of Keybox against the isolated synthetic keyrings from
# `cargo run --example dev_env`, browses, searches, reveals, copies, creates,
# edits and deletes entries and keyrings, unlocks and locks, then checks that
# none of the distinctive fixture values appear in Keybox's output or saved
# preferences.
#
# Needs a Wayland/X11 session: a Keybox window opens for about a minute.
# Uses a temporary XDG_CONFIG_HOME, so your real Keybox settings are untouched.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --quiet --bin cosmic-ext-keybox --example dev_env

work=$(mktemp -d)
cleanup() {
  # Closing its input or SIGTERM makes dev_env stop its daemons and exit.
  exec 3>&- 2>/dev/null || true
  if [[ -n "${env_pid:-}" ]]; then
    kill -TERM "$env_pid" 2>/dev/null || true
    wait "$env_pid" 2>/dev/null || true
  fi
  [[ -n "${KEEP:-}" ]] && echo "kept $work" || rm -rf -- "$work"
}
trap cleanup EXIT

mkfifo "$work/env.in"
target/debug/examples/dev_env < "$work/env.in" > "$work/env.out" 2> "$work/env.err" &
env_pid=$!
exec 3> "$work/env.in"
for _ in $(seq 1 60); do
  grep -q KEYBOX_DEV_BUS "$work/env.out" 2>/dev/null && break
  sleep 1
done
bus=$(sed -n 's/^KEYBOX_DEV_BUS=//p' "$work/env.out")
[[ -n "$bus" ]] || { echo "dev_env did not start"; cat "$work/env.err"; exit 1; }

script="wait:2500;search:alice;wait:500;search-close;select:KBX Alpha;reveal;wait:800"
script+=";copy;wait:500;select:KBX Control;reveal;wait:500;copy;wait:500;confirm"
script+=";select:KBX Binary;reveal;wait:500;keyring:Work;unlock;wait:2000"
script+=";keyring:Login;new-entry;label=KBX Typed entry;add-attr;attr-key0=kbx-typed-key"
script+=";attr-value0=kbx-typed-value;secret=KBX-FIXTURE-TYPED;show-secret;save;wait:1500"
script+=";edit-entry;label=KBX Typed edited;replace-secret;secret=KBX-FIXTURE-RETYPED;save"
script+=";wait:1500;delete-entry;confirm;wait:1000"
script+=";new-keyring;dialog=KBX Typed keyring;confirm;wait:2000"
script+=";rename-keyring;dialog=KBX Typed renamed;confirm;wait:1000;delete-keyring;confirm;wait:1000"
script+=";keyring:Login;lock;confirm;wait:1000;settings;reveal-choice:2;wait:500;quit"

mkdir -p "$work/config"
KEYBOX_DEV_BUS="$bus" KEYBOX_DEV_SCRIPT="$script" XDG_CONFIG_HOME="$work/config" RUST_LOG=trace \
  timeout 120 target/debug/cosmic-ext-keybox > "$work/app.log" 2>&1 || echo "keybox exited with status $?"

needles=(
  KBX-FIXTURE "KBX Alpha" "KBX Duplicate" "KBX Work" alice example.invalid
  "KBX Typed" kbx-typed-key kbx-typed-value
  kbx-test-login-password kbx-test-work-password
)
status=0
for needle in "${needles[@]}"; do
  if grep -rqF -- "$needle" "$work/app.log" "$work/config"; then
    echo "LEAK: found \"$needle\""
    status=1
  fi
done

if [[ -z "$(find "$work/config" -name reveal_seconds -type f)" ]]; then
  echo "WARNING: no saved preferences were written; the settings check did not run"
  status=1
fi

if [[ $status -eq 0 ]]; then
  echo "OK: no fixture data in output ($(wc -l < "$work/app.log") lines) or saved preferences"
fi
exit $status
