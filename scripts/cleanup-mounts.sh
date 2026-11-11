#!/usr/bin/env bash
# Tear down the host-side mounts created by setup-mounts.sh (PRD 13).
#
# Stop the Compose stack first ("docker compose down"); this script then
# unmounts the FUSE filesystem before the mount directories are removed.
# Safe to re-run.
#
# Usage (as root, from the directory that contains ./filtered):
#   sudo ../../scripts/cleanup-mounts.sh [dir]
set -euo pipefail

fail() { printf 'cleanup-mounts: %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || fail "run as root (unmounting needs root)"
BASE="$(cd "${1:-.}" && pwd)/filtered"
[ -d "$BASE" ] || exit 0

# Unmount the FUSE mount before the directories go away (lazy, so a dead
# "Transport endpoint is not connected" mount or a busy mount still detaches).
mountpoint -q "$BASE/target" && umount -l "$BASE/target"
mountpoint -q "$BASE" && umount -l "$BASE"
rmdir "$BASE/target" "$BASE" 2>/dev/null || true
echo "cleanup-mounts: $BASE cleaned up"
