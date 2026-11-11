#!/usr/bin/env bash
# Prepare the host-side shared mountpoint for the Scrim Compose example (PRD 5.2).
#
# Makes <dir>/filtered a shared (rshared) bind mount so the FUSE filesystem
# scrim mounts inside its container propagates back to the host, where the
# Docker daemon can bind-mount it into application containers.
#
# Idempotent: also clears a stale FUSE mount left by a previous run
# ("Transport endpoint is not connected" after an unclean shutdown).
#
# Usage (as root, from the directory that contains ./filtered):
#   sudo ../../scripts/setup-mounts.sh [dir]
set -euo pipefail

fail() { printf 'setup-mounts: %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || fail "run as root (mount propagation setup needs mount(8))"
BASE="$(cd "${1:-.}" && pwd)/filtered"
mkdir -p "$BASE/target"

# Clear a stale FUSE mount from a previous run; scrim refuses to mount over one.
if findmnt -T "$BASE/target" -n -o FSTYPE 2>/dev/null | grep -q '^fuse'; then
    umount -l "$BASE/target"
fi

mountpoint -q "$BASE" || mount --bind "$BASE" "$BASE"
mount --make-rshared "$BASE"
findmnt -T "$BASE" -o TARGET,PROPAGATION
