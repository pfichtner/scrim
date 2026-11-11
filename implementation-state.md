# docker-fuse-filter Implementation State

## Current Status (Tue Nov 10 2026)

- **Phase 0 (Feasibility):** ✅ COMPLETE — FUSE → host → application mount propagation verified on native Linux Docker Engine with `scripts/phase0-feasibility.sh`. The containerized architecture is viable; no host-side supervisor fallback is needed.
- **M1 (Rust FUSE passthrough):** ✅ COMPLETE — inode-based passthrough implemented with `fuser` 0.18 and manually verified on the host (mount, read, write-through, mkdir/rename/delete, read-only mode, unmount, healthcheck).
- **M2/M3 implementation:** Largely landed — exclusion rules enforced in all filesystem operations, path traversal and symlink escapes rejected, write-through works. Integration/security test suite still pending.
- **Dependencies:** Rust 1.99.1; `fuser` 0.18 (pure Rust, no libfuse needed). `cargo test --locked` passes 18 unit tests; fmt/clippy clean.

---

## Project Structure (per PRD §12)

```
docker-fuse-filter/
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
sudo target/debug/docker-fuse-filter mount --source /tmp/scrim-src \
  --mountpoint /tmp/scrim-mnt --config /tmp/scrim.yaml
ls /tmp/scrim-mnt                       # source contents, excluded entries omitted
cat /tmp/scrim-mnt/hello.txt            # works
cat /tmp/scrim-mnt/.env                 # ENOENT
echo hi > /tmp/scrim-mnt/new.txt        # appears in source immediately
sudo target/debug/docker-fuse-filter healthcheck --mountpoint /tmp/scrim-mnt
sudo target/debug/docker-fuse-filter unmount --mountpoint /tmp/scrim-mnt
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

## Milestone 4: Docker Image, Mount Propagation & Compose Integration (M4)

**Goal:** Containerize and wire up Docker Compose with mount propagation.

**Pending:**
- Dockerfile: multi-stage build (Rust → distroless/minimal Debian), install fuse3, non-root user
- mount propagation scripts:
  - `scripts/setup-mounts.sh`: create shared mount `rshared`
  - `scripts/cleanup-mounts.sh`: safe unmount then `rprivate`
- docker-compose.yml (per PRD §5.1):
  - `fuse-filter` service: `cap_add: SYS_ADMIN`, `devices: /dev/fuse`, `security_opt: apparmor:unconfined`
  - Volumes: `./project:/source`, `./filtered:/filtered` (rshared)
  - `application` service: `depends_on: fuse-filter (service_healthy)`, volume `./filtered/target:/workspace` (rslave)
- Healthcheck integration
- E2E test on native Linux: `./scripts/setup-mounts.sh && docker compose up -d`

**Deliverable (M4):** AC-001 through AC-014 pass on native Linux Docker Engine.

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

## Quick Next Steps (Phase 0 done)

1. **M1:** ✅ FUSE passthrough with `fuser` 0.18 and CLI exit codes; verified on host
2. **M2/M3:** ✅ Enforcement and write-through implemented; add automated integration/security tests
3. **M4:** Dockerfile + Compose using the verified `rshared`/`rslave` model from Phase 0
4. **M5:** Security tests, failure handling, documentation

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
| `Dockerfile` | ❌ missing | Create per PRD |
| `docker-compose.yml` | ❌ missing | Create per PRD §5.1 |
| `scripts/phase0-feasibility.sh` | ✅ OK | Phase 0 propagation PoC; passes on native Linux Docker |
| `scripts/setup-mounts.sh` | ❌ missing | Create host mount prep |
| `scripts/cleanup-mounts.sh` | ❌ missing | Create host cleanup |

---

## Critical Path

1. **Phase 0 PoC** on native Linux → ✅ propagation model verified
2. **M1:** FUSE ops with `fuser` crate → ✅ implemented and manually verified on host
3. **M2:** Exclusion enforcement in every operation → ✅ implemented; automated integration/security tests pending
4. **M3:** Write-through + concurrency → ✅ implemented; integration tests pending
5. **M4:** Docker image + compose → next
6. **M5:** Security + failure + docs

---

## To Resume Later

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /workspace/docker-fuse-filter
cargo fmt --all
cargo clippy --locked --all-targets
cargo test --locked
```

Host mount smoke test (requires root and `/dev/fuse`):

```bash
mkdir -p /tmp/scrim-src /tmp/scrim-mnt
sudo target/debug/docker-fuse-filter mount --source /tmp/scrim-src \
  --mountpoint /tmp/scrim-mnt --config config/config.example.yaml
sudo target/debug/docker-fuse-filter healthcheck --mountpoint /tmp/scrim-mnt
sudo target/debug/docker-fuse-filter unmount --mountpoint /tmp/scrim-mnt
```

Next: add automated integration/security tests (M2/M5), then the Dockerfile,
Compose file, and mount setup/cleanup scripts (M4).