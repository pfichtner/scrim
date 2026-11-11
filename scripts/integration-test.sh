#!/usr/bin/env bash
# End-to-end Docker integration test for the Scrim Compose example (PRD 11.3).
# Covers AC-001..AC-014 against a rootful Docker daemon on native Linux.
#
# Bind-mount sources and mount-propagation prep must happen in the daemon's
# mount namespace, which may differ from the caller's view (dev containers
# using the host's docker socket). Paths are therefore mapped to the daemon's
# view via findmnt, and host-namespace operations (mount prep, host-side
# checks, unmounts) run through a privileged helper that enters PID 1's mount
# namespace. On a plain native host both resolve to the caller's own view.
#
# Run as root from the repository root:
#   sudo ./scripts/integration-test.sh
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
EX_REL=examples/basic
PROJECT=scrim-e2e
NS_IMAGE=debian:bookworm-slim
APP_MARKER=scrim-e2e-marker.txt
HOST_MARKER=scrim-e2e-host.txt
EXPECTED_JS='console.log("hello scrim");'

log() { printf '\n=== %s ===\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || fail "run as root"
docker info >/dev/null 2>&1 || fail "rootful Docker daemon not reachable"
docker compose version >/dev/null 2>&1 || fail "docker compose plugin not found"
[ -e /dev/fuse ] || fail "/dev/fuse not available (native Linux host required)"

# Run a command in PID 1's mount namespace (= the daemon's host view).
hostns() { docker run --rm --privileged --pid=host "$NS_IMAGE" nsenter -t 1 -m -- "$@"; }

# Map a caller-visible path to the same file as the Docker daemon sees it.
# Falls back to the caller's path when the candidate does not match by inode
# (plain native mounts, subvolume roots).
host_path() {
    local p="$1" src tgt root cand
    src="$(findmnt -T "$p" -n -o SOURCE)"
    tgt="$(findmnt -T "$p" -n -o TARGET)"
    case "$src" in
        *\[*\]) root="${src#*[}"; root="${root%]}" ;;
        *) printf '%s\n' "$p"; return ;;
    esac
    cand="$root${p#"$tgt"}"
    if [ "$(hostns stat -c '%d:%i' "$cand" 2>/dev/null)" = "$(stat -c '%d:%i' "$p")" ]; then
        printf '%s\n' "$cand"
    else
        printf '%s\n' "$p"
    fi
}

HOST_REPO="$(host_path "$REPO")"
HOST_EX="$HOST_REPO/$EX_REL"
EX="$REPO/$EX_REL"
dc() { docker compose -p "$PROJECT" -f "$EX/compose.yaml" --project-directory "$HOST_EX" "$@"; }
app() { dc exec -T application sh -c "$1"; }

cleanup() {
    dc down --remove-orphans >/dev/null 2>&1 || true
    hostns bash "$HOST_REPO/scripts/cleanup-mounts.sh" "$HOST_EX" >/dev/null 2>&1 || true
    rm -f "$EX/project/$APP_MARKER" "$EX/project/$HOST_MARKER"
}
trap cleanup EXIT

# Fresh state from any previous run
cleanup

log "Preparing host-side shared mountpoint (daemon mount namespace)"
hostns bash "$HOST_REPO/scripts/setup-mounts.sh" "$HOST_EX"

log "Building the scrim image"
docker build -q -t scrim:latest "$REPO" >/dev/null

log "Starting the stack (AC-001)"
dc up -d --no-build >/dev/null

scrim_id="$(dc ps -q scrim)"
healthy=""
for _ in $(seq 1 60); do
    status="$(docker inspect -f '{{.State.Health.Status}}' "$scrim_id" 2>/dev/null || echo unknown)"
    if [ "$status" = healthy ]; then healthy=1; break; fi
    if [ "$(docker inspect -f '{{.State.Running}}' "$scrim_id" 2>/dev/null)" != true ]; then
        docker logs "$scrim_id" >&2 || true
        fail "scrim container exited during startup"
    fi
    sleep 2
done
[ "$healthy" = 1 ] || fail "scrim did not become healthy"

log "FUSE mount propagated to the host (AC-002, AC-011)"
hostns findmnt -T "$HOST_EX/filtered/target" -n -o FSTYPE | grep -q '^fuse' \
    || fail "host does not see the FUSE mount at the shared mountpoint"
[ "$(hostns cat "$HOST_EX/filtered/target/index.js")" = "$EXPECTED_JS" ] \
    || fail "host cannot read through the propagated mount"

log "Application sees the filtered view (AC-003, AC-004, AC-010)"
listing="$(app 'ls -a /workspace')"
for entry in index.js package.json node_modules; do
    echo "$listing" | grep -qxF "$entry" || fail "expected $entry in /workspace listing"
done
for entry in .env .env.production .git private.key; do
    if echo "$listing" | grep -qxF "$entry"; then
        fail "$entry must not appear in /workspace listing"
    fi
done

log "Excluded files are inaccessible by exact name (AC-005)"
for entry in .env .env.production private.key; do
    if app "cat /workspace/$entry" >/dev/null 2>&1; then
        fail "application could read excluded file $entry"
    fi
done
app "cat /workspace/index.js" >/dev/null || fail "application cannot read a visible file"

log "Write-through in both directions (AC-007, AC-008)"
app "echo from-app > /workspace/$APP_MARKER"
grep -qxF from-app "$EX/project/$APP_MARKER" \
    || fail "application write did not reach the source directory"
echo from-host > "$EX/project/$HOST_MARKER"
[ "$(app "cat /workspace/$HOST_MARKER")" = from-host ] \
    || fail "host write was not visible to the application"

log "Application container is unprivileged (AC-009)"
app 'test ! -e /source' || fail "application container unexpectedly has /source"
app_id="$(dc ps -q application)"
caps="$(docker inspect -f '{{json .HostConfig.CapAdd}}' "$app_id")"
[ "$caps" = "[]" ] || [ "$caps" = "null" ] \
    || fail "application container unexpectedly has added capabilities: $caps"

log "Fail-safe: stopping scrim must not expose the source (AC-013)"
dc stop scrim >/dev/null
sleep 2
if app 'cat /workspace/index.js' >/dev/null 2>&1; then
    fail "application can still read after scrim stopped (must not fall back to source)"
fi
[ "$(cat "$EX/project/index.js")" = "$EXPECTED_JS" ] \
    || fail "source directory was modified by the failure"

log "PASS: AC-001..AC-014 verified (AC-006/012/014 covered by pattern set, health gate, and this Compose run)"
