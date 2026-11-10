#!/usr/bin/env bash
# Phase 0 feasibility proof (PRD §18 / §5.2).
#
# Proves: a FUSE mount created inside a container propagates to the host
# through a shared bind mount, and a second container then sees that mount
# through an rslave bind mount -- without SYS_ADMIN, /dev/fuse, or privileged
# mode. Also checks that killing the FUSE container does not expose the source.
#
# bindfs is used only as a minimal stand-in for the real FUSE filesystem.
# Filtering is implemented in M1/M2 and is deliberately NOT tested here.
#
# Requirements: native Linux host, rootful Docker Engine, run as root.
set -euo pipefail

BASE="${PHASE0_DIR:-/tmp/docker-fuse-filter-phase0}"
FUSE_CT=dff-phase0-fuse
APP_CT=dff-phase0-app

log() { printf '\n=== %s ===\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }

cleanup() {
    docker rm -f "$APP_CT" "$FUSE_CT" >/dev/null 2>&1 || true
    umount -l "$BASE/filtered/target" >/dev/null 2>&1 || true
    umount -l "$BASE/filtered" >/dev/null 2>&1 || true
    rm -rf "$BASE"
}
trap cleanup EXIT

[ "$(id -u)" -eq 0 ] || fail "run as root (mount propagation setup needs mount(8))"
command -v docker >/dev/null || fail "docker not found"
docker info >/dev/null 2>&1 || fail "rootful Docker daemon not reachable"

log "Preparing host-side shared mountpoint"
rm -rf "$BASE"
mkdir -p "$BASE/source/conf/secrets" "$BASE/filtered/target"
printf 'visible-content\n' > "$BASE/source/app.txt"
printf 'SECRET=do-not-expose\n' > "$BASE/source/.env"
printf '{}\n' > "$BASE/source/conf/database.json"
printf 'credentials\n' > "$BASE/source/conf/secrets/credentials.json"
chmod -R 0777 "$BASE"
mountpoint -q "$BASE/filtered" || mount --bind "$BASE/filtered" "$BASE/filtered"
mount --make-rshared "$BASE/filtered"
findmnt -T "$BASE/filtered" -o TARGET,PROPAGATION

log "Starting FUSE container (bindfs as the minimal FUSE stand-in)"
docker run -d --name "$FUSE_CT" \
    --cap-add SYS_ADMIN \
    --device /dev/fuse:/dev/fuse \
    --security-opt apparmor:unconfined \
    --mount "type=bind,source=$BASE/source,target=/source" \
    --mount "type=bind,source=$BASE/filtered,target=/filtered,bind-propagation=rshared" \
    debian:bookworm-slim \
    sh -c 'apt-get update -qq && apt-get install -y -qq fuse3 bindfs >/dev/null 2>&1 && bindfs /source /filtered/target && echo READY && sleep infinity' \
    >/dev/null

log "Waiting for the FUSE mount"
ready=0
for _ in $(seq 1 90); do
    if docker logs "$FUSE_CT" 2>&1 | grep -q READY; then ready=1; break; fi
    [ "$(docker inspect -f '{{.State.Running}}' "$FUSE_CT" 2>/dev/null || true)" = true ] \
        || fail "FUSE container exited during startup"
    sleep 2
done
[ "$ready" = 1 ] || fail "FUSE mount did not become ready"

log "Verifying the FUSE mount propagated to the HOST"
findmnt -T "$BASE/filtered/target" -o TARGET,FSTYPE
findmnt -T "$BASE/filtered/target" -o FSTYPE -n | grep -qx fuse \
    || fail "host does not see a FUSE mount at the shared mountpoint"
[ "$(cat "$BASE/filtered/target/app.txt")" = visible-content ] \
    || fail "host cannot read through the propagated mount"
echo from-host > "$BASE/filtered/target/host-wrote.txt"
[ "$(cat "$BASE/source/host-wrote.txt")" = from-host ] \
    || fail "host write did not reach the source directory"

log "Starting application container (no SYS_ADMIN, no /dev/fuse)"
docker run -d --name "$APP_CT" \
    --mount "type=bind,source=$BASE/filtered/target,target=/workspace,bind-propagation=rslave" \
    alpine:3 sleep infinity >/dev/null
sleep 1
docker inspect -f '{{json .HostConfig.CapAdd}}' "$APP_CT" | grep -qx null \
    || fail "application container unexpectedly has added capabilities"
[ "$(docker exec "$APP_CT" cat /workspace/app.txt)" = visible-content ] \
    || fail "application cannot read through the propagated mount"
docker exec "$APP_CT" sh -c 'test ! -e /dev/fuse' \
    || fail "application container unexpectedly has /dev/fuse"
docker exec "$APP_CT" sh -c 'echo from-app > /workspace/app-wrote.txt'
[ "$(cat "$BASE/source/app-wrote.txt")" = from-app ] \
    || fail "application write did not reach the source directory"

log "Verifying fail-safe: killing FUSE must not expose the source"
docker stop "$FUSE_CT" >/dev/null
sleep 1
if docker exec "$APP_CT" cat /workspace/app.txt >/dev/null 2>&1; then
    fail "application can still read after FUSE death (must not fall back to source)"
fi
[ "$(cat "$BASE/source/app.txt")" = visible-content ] \
    || fail "source directory was modified by the failure"

log "PASS: FUSE -> host -> application propagation and fail-safe verified"
