#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

test_status_prints_local_daily_context() (
  out="$(scripts/use-daily.sh --status)"
  printf '%s\n' "$out" | grep -Fq 'daily checkout:'
  printf '%s\n' "$out" | grep -Fq 'debug binaries:'
  printf '%s\n' "$out" | grep -Fq 'target/debug/pmtui'
  printf '%s\n' "$out" | grep -Fq 'start local daily:'
)

test_build_uses_locked_cargo_build() (
  local dir log
  dir="$(mktemp -d)"
  trap 'rm -rf "$dir"' RETURN
  log="$dir/cargo.log"
  cat >"$dir/cargo" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$@" > "$log"
EOF
  chmod +x "$dir/cargo"

  CARGO="$dir/cargo" scripts/use-daily.sh --build >/dev/null
  grep -qx 'build' "$log"
  grep -qx -- '--locked' "$log"
)

test_restart_daemon_writes_only_the_stop_request() (
  local dir registry before after
  dir="$(mktemp -d)"
  trap 'rm -rf "$dir"' RETURN
  registry="$dir/registry.json"
  printf '[]\n' >"$registry"

  before="$(find "$dir" -maxdepth 1 -type f | wc -l | tr -d ' ')"
  scripts/use-daily.sh --restart-daemon --registry "$registry" --socket daily-test >/dev/null
  after="$(find "$dir" -maxdepth 1 -type f | wc -l | tr -d ' ')"

  [ "$before" -eq 1 ]
  [ "$after" -eq 2 ]
  test -f "$dir"/pmd-stop-daily-test-*.lock
)

test_status_prints_local_daily_context
test_build_uses_locked_cargo_build
test_restart_daemon_writes_only_the_stop_request
printf 'use-daily tests: ok\n'
