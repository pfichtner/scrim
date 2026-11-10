# docker-fuse-filter Implementation State

## Current Status (Tue Nov 10 2026)

- **Phase 0 (Feasibility):** ✅ COMPLETE — FUSE → host → application mount propagation verified on native Linux Docker Engine with `scripts/phase0-feasibility.sh`. The containerized architecture is viable; no host-side supervisor fallback is needed.
- **Phase 1-4 (Rust implementation):** READY TO CONTINUE — project skeleton compiles cleanly with `cargo check` (0 errors).
- **Dependencies:** Installed fuse3, libfuse3-dev, Rust 1.99.1

---

## Project Structure (per PRD §12)

```
docker-fuse-filter/
├── Cargo.toml
├── Cargo.lock
├── Dockerfile      ← to create
├── docker-compose.yml   ← to create
├── README.md       ← to create
├── config/
│   └── config.example.yaml   ← to create
├── src/
│   ├── main.rs         ✅ compiled OK
│   ├── cli.rs          ✅ compiled OK
│   ├── config.rs       ✅ compiled OK
│   ├── filter.rs       ✅ compiled OK (globset pattern matching)
│   ├── filesystem.rs   ✅ compiled OK (passthrough stub)
│   ├── mount.rs        ✅ compiled OK (FUSE lifecycle stubs)
│   ├── health.rs       ✅ compiled OK (health check result types)
│   ├── errors.rs       ← to create
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

## Milestone 1: Rust FUSE Passthrough Filesystem (M1) — IN PROGRESS

**Goal:** Basic FUSE mount that forwards all operations to source directory on the host.

**Completed:**
- ✅ Project skeleton created
- ✅ `cargo check` passes (0 errors)
- ✅ Core modules: `main.rs`, `cli.rs`, `config.rs`, `filter.rs`, `filesystem.rs`, `mount.rs`, `health.rs`

**Pending from M1:**
- Implement FUSE passthrough using `fuser` crate 0.18
- All required ops: `lookup`, `readdir`, `getattr`, `open`, `read`, `write`, `create`, `mkdir`, `unlink`, `rmdir`, `rename`, `truncate`, `chmod`, `chown`, `utimens`, `statfs`
- CLI: `mount`, `healthcheck`, `validate`, `config show`, `unmount` subcommands with exit codes (0–5 per PRD §9)
- Health check: verify mountpoint is active FUSE mount
- Unit tests: mount temp dir, verify basic read/write/ls, unmount cleanup

**Deliverable (M1):** `cargo run -- mount --source /tmp/src --mountpoint /tmp/mnt` works on host; `ls /tmp/mnt` shows source contents.

---

## Milestone 2: Configurable Exclusion Patterns & Security Filtering (M2)

**Goal:** Hide files matching patterns in all filesystem operations.

**Completed so far:**
- ✅ `filter.rs`: `GlobSet`-based pattern matching with `case_sensitive` option
- ✅ Pattern support: `.env`, `.env.*`, `*.pem`, `*.key`, `.git/`, `**/secrets/**`
- ✅ Unit tests in `filter.rs`: basic exclusion, wildcard, directory recursive

**Pending from M2:**
- Enforce filtering in EVERY FUSE operation (PRD §4.2 table):
  - `lookup` → reject excluded paths (ENOENT)
  - `readdir` → omit excluded entries
  - `getattr` → reject excluded paths (ENOENT)
  - `open` → reject excluded files (ENOENT)
  - `read`/`write` → forward only if `open` succeeded
  - `create`/`mkdir` → allow only if parent visible AND new name not excluded
  - `unlink`/`rmdir` → allow only if target visible
  - `rename` → enforce filtering on BOTH source and destination
- Path resolution security (PRD §6.2):
  - Reject `..` traversals and absolute-path escapes
  - Reject symlinks escaping source tree or pointing to excluded targets
  - Explicit design + test for hard-link behavior
- Config validation at startup (PRD §8.2):
  - Source directory must exist
  - Mountpoint must not already be mounted
  - Config YAML validation
  - Environment-variable overrides
- Exit code support per PRD §9

**Security-critical:** All ops must enforce exclusion rules — `readdir` filtering alone is insufficient (PRD §6.1).

**Deliverable (M2):** Unit tests pass for all filtering cases in PRD §11.1–11.2 on host FUSE mount.

---

## Milestone 3: Writable Filesystem & Full Operation Support (M3)

**Goal:** Complete read/write semantics with write-through to source.

**Pending from M3:**
- Write operations: forward directly to source fd (no buffering/copying)
- `create` → `openat(source_fd, name, O_CREAT|O_EXCL, mode)` on source
- `mkdir` → `mkdirat(source_fd, name, mode)`
- `unlink`/`rmdir` → on source
- `truncate` → `ftruncate` on source fd
- `chmod`/`chown`/`utimens` → forward via `fchmodat`/`fchownat`/`futimensat`
- Concurrency handling (FUSE is multi-threaded)
- Large files: stream — no full-file reads into memory
- Edge cases: rename with excluded descendants, symlink creation restrictions

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

1. **M1:** Implement the FUSE passthrough with `fuser` 0.18 and CLI exit codes; verify on host
2. **M2:** Enforce exclusion patterns in every FUSE operation (pure Rust, no Docker needed)
3. **M3:** Write-through and full operation support
4. **M4:** Dockerfile + Compose using the verified `rshared`/`rslave` model from Phase 0
5. **M5:** Security tests, failure handling, documentation

---

## Key Files Already Created

| File | Status | Notes |
|------|--------|-------|
| `Cargo.toml` | ✅ OK | deps: anyhow 1.0, clap 4.5, serde 1.0, globset 0.4, fuser 0.18, tokio 1.38 |
| `src/main.rs` | ✅ OK | CLI parser with 5 subcommands |
| `src/cli.rs` | ✅ OK | mount/healthcheck/validate/config_show/unmount functions |
| `src/config.rs` | ✅ OK | FilesystemConfig, FilterConfig, LoggingConfig, Config with serde |
| `src/filter.rs` | ✅ OK | GlobSet pattern matching, unit tests |
| `src/filesystem.rs` | ✅ OK | PassthroughFileskeleton struct, FuserFile helper |
| `src/mount.rs` | ✅ OK | MountContext, mount/healthcheck/unmount stubs |
| `src/health.rs` | ✅ OK | HealthStatus, HealthCheckResult types |
| `src/errors.rs` | ❌ missing | Create for error type hierarchy |
| `src/lib.rs` | ❌ missing | Optional re-exports |
| `Dockerfile` | ❌ missing | Create per PRD |
| `docker-compose.yml` | ❌ missing | Create per PRD §5.1 |
| `scripts/phase0-feasibility.sh` | ✅ OK | Phase 0 propagation PoC; passes on native Linux Docker |
| `scripts/setup-mounts.sh` | ❌ missing | Create host mount prep |
| `scripts/cleanup-mounts.sh` | ❌ missing | Create host cleanup |

---

## Critical Path

1. **Phase 0 PoC** on native Linux → ✅ propagation model verified
2. **M1:** Implement FUSE ops with `fuser` crate → pass host unit tests
3. **M2:** Enforce exclusion in every FUSE operation → pass PRD §11.1–11.2 unit tests
4. **M3:** Write-through + concurrency → pass PRD §11.2 integration tests on host
5. **M4:** Docker image + compose → pass AC-001..AC-014 on native Linux
6. **M5:** Security + failure + docs → pass AC-001..AC-015

---

## To Resume Later

Run:
```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /workspace/docker-fuse-filter
cargo check   # verifies: 0 errors
```

Then proceed based on Phase 0 PoC result.