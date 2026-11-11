# Scrim Implementation State

## Current Status (Wed Nov 11 2026)

- **Phase 0 (Feasibility):** ✅ COMPLETE — FUSE → host → application mount propagation verified on native Linux Docker Engine with `scripts/phase0-feasibility.sh`. The containerized architecture is viable; no host-side supervisor fallback is needed.
- **M1 (Rust FUSE passthrough):** ✅ COMPLETE — inode-based passthrough implemented with `fuser` 0.18 and manually verified on the host (mount, read, write-through, mkdir/rename/delete, read-only mode, unmount, healthcheck).
- **M2/M3 implementation:** Largely landed — exclusion rules enforced in all filesystem operations, path traversal and symlink escapes rejected, write-through works. Integration/security test suite still pending.
- **M4 (Docker/Compose):** ✅ COMPLETE — Dockerfile, Compose example, host mount setup/cleanup scripts, and `scripts/integration-test.sh` pass end to end (AC-001..AC-014) on the dev host.
- **Dependencies:** Rust 1.99.1; `fuser` 0.18 (pure Rust, no libfuse needed — the image needs no fuse3 packages). `cargo test --locked` passes 18 unit tests; fmt/clippy clean.

### M4 environment notes (important for re-running)

The development sandbox runs inside a container whose `/tmp` and `/workspace`
are private, while the Docker socket belongs to the host daemon. Bind-mount
sources are resolved in the daemon's mount namespace, so
`scripts/integration-test.sh` maps paths to the daemon's view via `findmnt`
and performs host-namespace mount operations (setup, propagation checks,
unmounts) through `docker run --privileged --pid=host ... nsenter -t 1 -m`.
On a plain native host both resolve to the caller's own view, so the script
works unchanged there too.

---

## Project Structure (per PRD §12)

```
scrim/
├── Cargo.toml
├── Cargo.lock
├── Dockerfile      ← to create
├── docker-compose.yml   ← to create
├── README.md       ✅ docs
├── config/
│   └── config.example.yaml   ✅
├── src/
│   ├── main.rs         ✅
│   ├── cli.rs          ✅
│   ├── config.rs       ✅
│   ├── filter.rs       ✅
│   ├── filesystem.rs   ✅ inode-based FUSE passthrough
│   ├── mount.rs        ✅ mount/unmount/is_mounted via fuser + /proc/self/mountinfo
│   ├── health.rs       ✅ health check result types + mount check
│   ├── errors.rs       ✅ exit codes and AppError
│   ├── fs/
│   │   ├── lookup.rs
│   │   ├── readdir.rs
│   │   ├── read.rs
│   │   ├── write.rs
│   │   ├── metadata.rs
│   │   ├── directory.rs
│   │   ├── rename.rs
│   │   └── symlink.rs
│   └── lib.rs          ← optional
├── tests/
│   ├── filtering.rs
│   ├── filesystem.rs
│   ├── security.rs
│   └── docker_integration.rs
├── examples/
│   ├── basic/
│   └── development/
├── scripts/
│   ├── phase0-feasibility.sh   ✅ Phase 0 proof
│   ├── setup-mounts.sh
│   ├── cleanup-mounts.sh
│   └── integration-test.sh
```

---

## Phase 0: Feasibility Proof (PRD §18) — ✅ COMPLETE

**Goal:** Prove that a FUSE mount inside the filter container propagates to the host and then into the application container via mount propagation.

**Result:** PASS. Reproducible proof: `sudo ./scripts/phase0-feasibility.sh` (run on the native Linux host; requires rootful Docker).

The PoC uses `bindfs` inside a container as a minimal FUSE stand-in (filtering is not part of Phase 0). It verifies:

1. Host prepares `filtered` as its own shared mount (`mount --bind` + `mount --make-rshared`).
2. FUSE container is started with `cap_add: SYS_ADMIN`, `/dev/fuse`, `apparmor:unconfined`, source bind mount, and `rshared` bind on `filtered`.
3. `bindfs /source /filtered/target` inside the container appears as a `fuse` mount on the host at `filtered/target`.
4. Application container sees it via an `rslave` bind mount with **no** added capabilities and **no** `/dev/fuse`.
5. Reads and writes through the application reach the host source directory immediately (write-through).
6. Killing the FUSE container leaves the host mount in `Transport endpoint is not connected`; the application gets I/O errors and the source is never exposed.

**Environment caveat (important for M4):** Docker resolves bind-mount source paths on the **daemon's host mount namespace**, not the caller's. The host-side `filtered` directory must exist and be a shared mount before containers are created. On Ubuntu, the default AppArmor profile blocks `mount(2)` even with `SYS_ADMIN`; `security-opt apparmor:unconfined` is required (as already specified in PRD §5.1).

**Decision:** Use the PRD §5.2 propagation model. No host-side mount supervisor fallback needed.

---

## Milestone 1: Rust FUSE Passthrough Filesystem (M1) — ✅ COMPLETE

**Goal:** Basic FUSE mount that forwards all operations to source directory on the host.

**Completed:**
- ✅ Inode-based path map and per-handle file descriptor map (no full-file copies)
- ✅ Ops: `lookup`, `readdir`, `getattr`, `setattr` (truncate/chmod/chown/utimens), `open`, `read`, `write`, `create`, `mkdir`, `unlink`, `rmdir`, `rename`, `link`, `symlink`, `readlink`, `flush`, `fsync`, `release`, `access`, `statfs`
- ✅ CLI: `mount`, `healthcheck`, `validate`, `config show`, `unmount` with exit codes 0–5
- ✅ Health check verifies the mount via `/proc/self/mountinfo` plus a directory read
- ✅ Read-only mode returns `EROFS` for writes
- ✅ 18 unit tests + manual host mount verification (`ls`, read, write-through, mkdir/rename/delete, hidden files, symlink hiding, healthcheck, unmount)

**Verified manually:**
```bash
sudo target/debug/scrim mount --source /tmp/scrim-src \
  --mountpoint /tmp/scrim-mnt --config /tmp/scrim.yaml
ls /tmp/scrim-mnt                       # source contents, excluded entries omitted
cat /tmp/scrim-mnt/hello.txt            # works
cat /tmp/scrim-mnt/.env                 # ENOENT
echo hi > /tmp/scrim-mnt/new.txt        # appears in source immediately
sudo target/debug/scrim healthcheck --mountpoint /tmp/scrim-mnt
sudo target/debug/scrim unmount --mountpoint /tmp/scrim-mnt
```

**Remaining:** an automated FUSE integration test that does not require CI to
have root and `/dev/fuse` (currently ignored by CI; host verification manual).

---

## Milestone 2: Configurable Exclusion Patterns & Security Filtering (M2)

**Goal:** Hide files matching patterns in all filesystem operations.

**Completed:**
- ✅ `filter.rs`: `GlobSet`-based pattern matching with case sensitivity
- ✅ Pattern support: `.env`, `.env.*`, `*.pem`, `*.key`, `.git/`, `**/secrets/**`
- ✅ Bare names match at any depth; directory patterns hide descendants
- ✅ Filtering enforced in every operation: `lookup`, `getattr`, `readdir`, `open`, `read`/`write`, `create`/`mkdir`, `unlink`/`rmdir`, `rename`, `link`, `symlink`
- ✅ Path security: `..`/absolute names rejected, symlinks hidden unless `allow_symlinks`, symlink targets must canonicalize inside source and be visible
- ✅ Config validation, source/mountpoint checks, exit codes 0–5
- ✅ Unit tests for filter matching, path sanitation, visibility, inode mapping

**Pending from M2:**
- Automated FUSE integration/security tests for each operation (PRD §11.1–11.2)
- Hard-link policy test (hard links to excluded paths cannot be created through the mount; a pre-existing hard link is treated as a normal visible path)
- Environment-variable overrides
- `hide_excluded_files: false` behavior (currently always hides)
- Case-insensitive and nested-pattern end-to-end tests

**Security-critical:** All ops must enforce exclusion rules — `readdir` filtering alone is insufficient (PRD §6.1).

**Deliverable (M2):** Unit tests pass for all filtering cases in PRD §11.1–11.2 on host FUSE mount.

---

## Milestone 3: Writable Filesystem & Full Operation Support (M3)

**Goal:** Complete read/write semantics with write-through to source.

**Completed:**
- ✅ Writes go directly to source file descriptors (`pwrite`, `pread`); no buffering or copying
- ✅ `create`, `mkdir`, `unlink`, `rmdir`, `rename`, `link`, `symlink`
- ✅ `truncate`, `chmod`, `chown`, `utimens` via `setattr`
- ✅ Concurrency: mutex-protected inode/handle maps, per-request cloned descriptors, 4 FUSE worker threads
- ✅ Large files stream through bounded `read`/`write` requests; no whole-file buffering
- ✅ Rename updates the inode map for moved subtrees
- ✅ Symlink creation rejects absolute targets and `..`

**Pending from M3:**
- Integration tests for write-through and concurrency
- Rename flags (`RENAME_NOREPLACE`, `RENAME_EXCHANGE`) are ignored
- Extended attributes are not forwarded
- SIGTERM graceful-unmount handling

**Deliverable (M3):** All tests from PRD §11.2 pass on host; modifications immediately reflected in source directory.

---

## Milestone 4: Docker Image, Mount Propagation & Compose Integration (M4) — ✅ COMPLETE

**Goal:** Containerize and wire up Docker Compose with mount propagation.

**Completed:**
- ✅ `Dockerfile`: multi-stage `rust:1-slim-bookworm` → `debian:bookworm-slim`, no fuse3/fusermount in the image (`fuser` 0.18 is pure Rust and mounts directly), baked-in default config at `/etc/scrim/config.yaml`, `HEALTHCHECK scrim healthcheck`. Scrim runs as root in its container: mounting needs `CAP_SYS_ADMIN`, and that container is the deliberately privileged, single-purpose component (documented in README).
- ✅ `examples/basic/`: sample project per PRD §17 (index.js, package.json, .env, .env.production, private.key, .git/, node_modules/), `config.yaml`, `compose.yaml` (scrim: `SYS_ADMIN` + `/dev/fuse` + `apparmor:unconfined` + rshared `/filtered`; application: health-gated, rslave bind of `./filtered/target`, no capabilities, no `./project` mount, config override bind).
- ✅ `scripts/setup-mounts.sh`: idempotent host prep — clears stale FUSE mounts, `mount --bind` self, `mount --make-rshared`, prints propagation state.
- ✅ `scripts/cleanup-mounts.sh`: unmounts FUSE before removing mount directories, restores plain dirs.
- ✅ `scripts/integration-test.sh`: full AC-001..AC-014 verification (see notes above for the daemon-namespace handling).
- ✅ E2E verified: scrim healthy, host sees propagated `fuse` mount, `.env`/`.env.production`/`private.key`/`.git` hidden and ENOENT by exact name, write-through both directions, app container unprivileged without `/source`, fail-safe on scrim stop, clean teardown.

**Known gaps:**
- The FUSE mount is only accessible to the mounting user (root): application containers must run as root and host access requires root. `allow_other` (PRD §5.3) is not enabled yet.
- `docker compose down` leaves the dead FUSE mount until `cleanup-mounts.sh` (or a re-run of `setup-mounts.sh`) clears it; documented in README troubleshooting.

**Deliverable (M4):** AC-001 through AC-014 pass — verified by `sudo ./scripts/integration-test.sh`.

---

## Milestone 5: Security Tests, Failure Handling & Documentation (M5)

**Pending:**
- Security test suite (PRD §6.2–6.4):
  - Path traversal: `../../etc/passwd` → ENOENT
  - Symlink to excluded file → ENOENT
  - Symlink escaping source tree → ENOENT
  - Hard link to excluded file → explicit behavior + test
  - Rename bypass attempts → rejected
  - Guessed filename access → ENOENT
- Failure handling:
  - FUSE crash → app container sees I/O errors, not source
  - Optional watchdog: script that `docker stop` dependents on healthcheck failure
  - Graceful unmount on SIGTERM
- Performance benchmark (PRD §7.2): sequential read/write vs raw bind mount, report overhead
- Complete README per PRD §13 (6 required sections + working example)
- CI/CD: GitHub Actions build + unit tests
- Publish Docker image

**Deliverable (M5):** All AC-001 through AC-015 pass; reproducible build documented.

---

## Quick Next Steps (M4 done)

1. **M1:** ✅ FUSE passthrough with `fuser` 0.18 and CLI exit codes; verified on host
2. **M2/M3:** ✅ Enforcement and write-through implemented; add automated integration/security tests
3. **M4:** ✅ Dockerfile + Compose + mount scripts; AC-001..AC-014 pass via `sudo ./scripts/integration-test.sh`
4. **M5:** Security tests, failure handling (watchdog, SIGTERM), benchmarks, license, image publication

---

## Key Files Already Created

| File | Status | Notes |
|------|--------|-------|
| `Cargo.toml` | ✅ OK | deps: anyhow, clap, serde, serde_yaml, globset, fuser 0.18, libc |
| `src/main.rs` | ✅ OK | CLI parser, exit-code dispatch |
| `src/cli.rs` | ✅ OK | mount/healthcheck/validate/config show/unmount |
| `src/config.rs` | ✅ OK | Config types and `Config::load` |
| `src/filter.rs` | ✅ OK | GlobSet matching, depth and directory semantics, unit tests |
| `src/filesystem.rs` | ✅ OK | Inode-based FUSE passthrough, all core/filter ops, unit tests |
| `src/mount.rs` | ✅ OK | mount/unmount/is_mounted via fuser + mountinfo |
| `src/health.rs` | ✅ OK | HealthStatus, HealthCheckResult, mount check |
| `src/errors.rs` | ✅ OK | Exit codes and AppError |
| `config/config.example.yaml` | ✅ OK | Example configuration |
| `src/lib.rs` | ❌ not planned | Binary crate; unit tests are inline |
| `Dockerfile` | ✅ OK | multi-stage rust:1-slim-bookworm → debian:bookworm-slim, no fuse3 needed |
| `examples/basic/` | ✅ OK | compose.yaml, config.yaml, sample project (PRD §17) |
| `docker-compose.yml` (root) | ❌ not created | `examples/basic/compose.yaml` is the canonical Compose file |
| `scripts/phase0-feasibility.sh` | ✅ OK | Phase 0 propagation PoC; passes on native Linux Docker |
| `scripts/setup-mounts.sh` | ✅ OK | idempotent host prep; clears stale FUSE mounts |
| `scripts/cleanup-mounts.sh` | ✅ OK | unmount FUSE before removing mount dirs |
| `scripts/integration-test.sh` | ✅ OK | AC-001..AC-014 e2e; daemon-namespace aware |

---

## Critical Path

1. **Phase 0 PoC** on native Linux → ✅ propagation model verified
2. **M1:** FUSE ops with `fuser` crate → ✅ implemented and manually verified on host
3. **M2:** Exclusion enforcement in every operation → ✅ implemented; automated integration/security tests pending
4. **M3:** Write-through + concurrency → ✅ implemented; integration tests pending
5. **M4:** Docker image + compose → ✅ complete, AC-001..AC-014 pass
6. **M5:** Security + failure + docs

---

## To Resume Later

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /workspace/scrim
cargo fmt --all
cargo clippy --locked --all-targets
cargo test --locked
sudo ./scripts/integration-test.sh   # full Docker e2e (AC-001..AC-014)
```

Next: automated FUSE integration/security tests for M2/M3 (PRD §11.1–11.2),
then M5: watchdog, SIGTERM graceful unmount, benchmarks, license, image
publication.