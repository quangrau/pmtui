#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REGISTRY="${HOME:?HOME is required}/.config/pmd/registry.json"
SOCKET="pmd"
ACTION="status"
ACTION_SET=0

usage() {
  cat <<'EOF'
Usage: scripts/use-daily.sh [--status|--build|--restart-daemon] [options]

Prepare or inspect a local daily checkout without touching releases.

Actions:
  --status          show checkout, debug binaries, active commands, and run command
  --build           run cargo build --locked in this checkout
  --restart-daemon  ask the pmd for this registry/socket to exit; project terminals stay running

Options:
  --registry PATH   registry JSON (default: ~/.config/pmd/registry.json)
  --socket NAME     tmux socket name (default: pmd)
  -h, --help        show this help
EOF
}

set_action() {
  if [ "$ACTION_SET" -eq 1 ]; then
    echo "only one action can be selected" >&2
    exit 2
  fi
  ACTION="$1"
  ACTION_SET=1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --status)
      set_action status
      ;;
    --build)
      set_action build
      ;;
    --restart-daemon)
      set_action restart_daemon
      ;;
    --registry)
      [ "$#" -ge 2 ] || { echo "--registry requires a path" >&2; exit 2; }
      REGISTRY="$2"
      shift
      ;;
    --socket)
      [ "$#" -ge 2 ] || { echo "--socket requires a name" >&2; exit 2; }
      SOCKET="$2"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

case "$REGISTRY" in
  /*) ;;
  *) REGISTRY="$PWD/$REGISTRY" ;;
esac

git_value() {
  git -C "$ROOT" "$@" 2>/dev/null || true
}

binary_state() {
  local path="$1"
  if [ -x "$path" ]; then
    printf 'ready   %s\n' "$path"
  elif [ -e "$path" ]; then
    printf 'present %s (not executable)\n' "$path"
  else
    printf 'missing %s\n' "$path"
  fi
}

print_processes() {
  local name="$1" pid comm command base found=0
  if ! command -v ps >/dev/null 2>&1; then
    printf 'active %s commands:\n  ps is not available\n' "$name"
    return
  fi
  printf 'active %s commands:\n' "$name"
  while read -r pid comm; do
    base="${comm##*/}"
    if [ "$base" = "$name" ]; then
      command="$(ps -p "$pid" -o command= 2>/dev/null || true)"
      printf '  %s %s\n' "$pid" "${command:-$comm}"
      found=1
    fi
  done < <(ps -axo pid=,comm=)
  if [ "$found" -eq 0 ]; then
    printf '  none found\n'
  fi
}

print_start_command() {
  printf 'start local daily:\n'
  printf '  %q --registry %q --socket %q\n' "$ROOT/target/debug/pmtui" "$REGISTRY" "$SOCKET"
}

cargo_command() {
  if [ -n "${CARGO:-}" ]; then
    printf '%s\n' "$CARGO"
  elif command -v cargo >/dev/null 2>&1; then
    command -v cargo
  elif [ -x "$HOME/.cargo/bin/cargo" ]; then
    printf '%s\n' "$HOME/.cargo/bin/cargo"
  else
    printf '%s\n' cargo
  fi
}

print_status() {
  local branch head upstream dirty counts behind ahead
  branch="$(git_value branch --show-current)"
  [ -n "$branch" ] || branch="detached"
  head="$(git_value rev-parse --short HEAD)"
  upstream="$(git_value rev-parse --abbrev-ref --symbolic-full-name '@{u}')"
  dirty="clean"
  if [ -n "$(git_value status --porcelain)" ]; then
    dirty="dirty"
  fi

  printf 'daily checkout:\n'
  printf '  root: %s\n' "$ROOT"
  printf '  branch: %s\n' "$branch"
  printf '  head: %s\n' "${head:-unknown}"
  printf '  worktree: %s\n' "$dirty"
  if [ -n "$upstream" ]; then
    counts="$(git_value rev-list --left-right --count '@{u}...HEAD')"
    behind="${counts%%[[:space:]]*}"
    ahead="${counts##*[[:space:]]}"
    printf '  upstream: %s (ahead %s, behind %s)\n' "$upstream" "${ahead:-0}" "${behind:-0}"
  else
    printf '  upstream: none\n'
  fi

  printf '\n'
  printf 'debug binaries:\n'
  printf '  '
  binary_state "$ROOT/target/debug/pmtui"
  printf '  '
  binary_state "$ROOT/target/debug/pmd"

  printf '\n'
  print_processes pmtui
  print_processes pmd

  printf '\n'
  print_start_command
}

run_build() {
  local cargo
  cargo="$(cargo_command)"
  (
    cd "$ROOT"
    "$cargo" build --locked
  )
  print_start_command
}

fnv64_hex() {
  perl -MMath::BigInt -e '
    my $hash = Math::BigInt->from_hex("0xcbf29ce484222325");
    my $prime = Math::BigInt->from_hex("0x100000001b3");
    my $mod = Math::BigInt->new(2)->bpow(64);
    local $/;
    my $value = <STDIN>;
    for my $byte (unpack("C*", $value)) {
      $hash->bxor($byte);
      $hash->bmul($prime);
      $hash->bmod($mod);
    }
    my $hex = $hash->as_hex();
    $hex =~ s/^0x//;
    print substr(("0" x 16) . $hex, -16), "\n";
  '
}

daemon_stop_path() {
  local registry="$1" socket="$2" root safe hash
  root="$(dirname "$registry")"
  safe="$(printf '%s' "$socket" | LC_ALL=C sed 's/[^A-Za-z0-9_-]/-/g')"
  [ -n "$safe" ] || safe="socket"
  hash="$(printf '%s\0%s' "$registry" "$socket" | fnv64_hex)"
  printf '%s/pmd-stop-%s-%s.lock\n' "$root" "$safe" "$hash"
}

restart_daemon() {
  local stop tmp
  stop="$(daemon_stop_path "$REGISTRY" "$SOCKET")"
  mkdir -p "$(dirname "$stop")"
  tmp="$stop.$$"
  # pmd owns the stop contract; the file asks it to exit between sweeps without touching project terminals.
  printf '%s\n' "$$" >"$tmp"
  mv -f "$tmp" "$stop"
  printf 'pmd stop requested: %s\n' "$stop"
  printf 'project terminals stay running; start the local dashboard when the daemon has exited.\n'
  print_start_command
}

case "$ACTION" in
  status) print_status ;;
  build) run_build ;;
  restart_daemon) restart_daemon ;;
esac
