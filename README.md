# Scrim

A filtered, writable filesystem for sandboxed applications and AI coding agents.

Scrim exposes a host directory through a FUSE filesystem that enforces configurable file and directory exclusions.

Applications can read, create, modify, and delete permitted files while excluded paths remain inaccessible.

Scrim supports Docker-based deployment and can also run directly on a Linux host.

Use Scrim to:
- Give AI coding agents access to source code without exposing .env files or credentials.
- Run untrusted development tools against real project directories.
- Restrict filesystem visibility for build systems and CI/CD jobs.
- Provide writable project access without exposing sensitive files.
Scrim supports Docker-based deployment and can also run directly on a Linux host.

## AI coding agents

AI coding agents often need broad access to a project directory.

They need to inspect source code, modify files, run tests, install
dependencies, and execute development tools. However, they generally
don't need access to deployment credentials, API keys, private SSH
keys, or other sensitive files stored alongside the source code.

Scrim provides a filtered filesystem view that lets an agent work
with the project while excluding explicitly configured paths.

For example:
```
project/                  /workspace/
├── src/                  ├── src/
├── tests/                ├── tests/
├── package.json          ├── package.json
├── README.md             └── README.md
├── .env
├── .env.production
├── .git/
├── secrets/
└── deploy.key
```

The coding agent receives access to `/workspace`, not the original
project directory.

Attempts to access excluded files directly return `ENOENT`.

Example configuration:

```
filter:
  patterns:
    - ".env"
    - ".env.*"
    - ".git/"
    - "*.key"
    - "*.pem"
    - "**/secrets/**"
```

The agent retains normal write access to permitted files. Changes
are immediately reflected in the original project directory.

Scrim does not provide a complete security sandbox. Network access,
process isolation, resource limits, and other attack surfaces must
be controlled separately.

## Why

Application containers often need a project directory mounted writable, but
that directory also contains files the application should never see:

```
/project/                     /workspace/ (visible to the application)
├── app.py                    ├── app.py
├── requirements.txt          ├── requirements.txt
├── config.json               └── config.json
├── .env
├── .env.production
├── private.key
└── .git/
```

Filtering `readdir` alone is not enough: a process can still open a file by its
exact name. Scrim enforces the exclusion policy on every filesystem operation,
so excluded paths fail with `ENOENT` instead of being readable.

## How it works

```
                 HOST
                   |
            /project (source)
                   | bind mount
                   v
       +----------------------------+
       | FUSE FILTER CONTAINER      |
       |                            |
       |   /source                  |
       |      |                     |
       |      v                     |
       |   FUSE FILESYSTEM          |
       |      |                     |
       |      v                     |
       |   /filtered/target         |
       +----------------------------+
                   |
            mount propagation
                   |
                   v
       +----------------------------+
       | APPLICATION CONTAINER      |
       |                            |
       |   /workspace               |
       |   (filtered view)          |
       +----------------------------+
```

Only the filter container receives `SYS_ADMIN` and `/dev/fuse`. The
application container only receives a normal bind mount.

The host-side `filtered` directory is prepared as a shared mount. The FUSE
container mounts under `/filtered/target`, the mount propagates to the host,
and the application container bind-mounts the propagated view. This sequence
was verified in Phase 0; see [`scripts/phase0-feasibility.sh`](scripts/phase0-feasibility.sh).

## Current status

| Area | Status |
| --- | --- |
| Mount propagation (FUSE -> host -> application) | Verified on native Linux Docker |
| FUSE passthrough filesystem | Implemented (M1), manually verified |
| Exclusion enforcement in all operations | Implemented (M2), integration tests pending |
| Write-through read/write support | Implemented (M3), integration tests pending |
| Docker image and Docker Compose example | Implemented (M4), e2e tested on native Linux |
| Security tests, failure watchdog, benchmarks | Planned (M5) |

See [`implementation-state.md`](implementation-state.md) for details.

## Prerequisites

- Linux with FUSE support (`/dev/fuse`)
- A rootful Docker Engine and Docker Compose for container usage
- Rust stable toolchain for building from source
- `sudo` access on the Docker host for mount preparation

## Build from source

```bash
cargo build --locked
```

Run the checks used by CI:

```bash
cargo fmt --all -- --check
cargo build --locked --all-targets
cargo clippy --locked --all-targets
cargo test --locked
```

## Verify mount propagation (Phase 0)

The feasibility script proves the architecture end to end using `bindfs` as a
minimal FUSE stand-in. It creates a temporary source directory, mounts it
through a container, confirms the mount is visible on the host, starts an
application container without extra capabilities, checks write-through, and
confirms that killing the FUSE container does not expose the source.

Run on the native Linux Docker host as root:

```bash
sudo ./scripts/phase0-feasibility.sh
```

The script cleans up its containers, mounts, and temporary directories when it
exits.

## CLI

The intended interface:

```bash
# Start the filesystem
scrim mount \
  --source /source \
  --mountpoint /filtered/target \
  --config /etc/scrim/config.yaml

# Check filesystem health
scrim healthcheck --mountpoint /filtered/target

# Validate configuration
scrim validate --config /etc/scrim/config.yaml

# Display active configuration
scrim config show --config /etc/scrim/config.yaml

# Unmount filesystem
scrim unmount --mountpoint /filtered/target
```

Exit codes are `0` success, `1` general error, `2` invalid configuration,
`3` source unavailable, `4` mount failure, and `5` filesystem unhealthy.
`--config` defaults to `/etc/scrim/config.yaml` for `mount`, `validate`,
and `config show`.

## Configuration

```yaml
filesystem:
  source: /source
  mountpoint: /filtered/target
  read_only: false
  allow_symlinks: false
  preserve_permissions: true

filter:
  patterns:
    - ".env"
    - ".env.*"
    - "*.pem"
    - "*.key"
    - ".git/"
    - "**/secrets/**"
  case_sensitive: true
  hide_excluded_files: true

logging:
  level: info
```

No exclusions are applied by default; every pattern is explicit. Defaults for
omitted fields are `read_only: false`, `allow_symlinks: false`,
`preserve_permissions: true`, `case_sensitive: true`,
`hide_excluded_files: true`, and `logging.level: info`.

### Pattern semantics

Patterns are matched against paths relative to the source root. Bare names
match at any depth (`.env` hides `config/.env`), and trailing `/` marks a
directory pattern. Matching uses `globset`, and the policy is enforced in
`lookup`, `getattr`, `readdir`, `open`, `read`, `write`, `create`, `mkdir`,
`unlink`, `rmdir`, `rename`, `link`, and `symlink`.

| Pattern | Intended meaning |
| --- | --- |
| `.env` | Hide files named `.env` |
| `.env.*` | Hide `.env.local`, `.env.production`, etc. |
| `*.pem` | Hide PEM files |
| `*.key` | Hide key files |
| `.git/` | Hide directories named `.git` and their contents |
| `**/secrets/**` | Hide contents below any `secrets` directory |

A trailing `/` marks a directory pattern and hides the directory itself plus
everything below it. `*` does not cross a path separator; `**` does. Negation
(`!pattern`) is not implemented yet. Case-insensitive matching is configurable.
`hide_excluded_files: false` is currently treated as `true`.

## Write-through behavior

Scrim is a passthrough filesystem, not a copy-on-write or staging layer. Reads
and writes operate directly on the source directory, so a change made through
the filtered mount is immediately visible on the host and no synchronization
process is involved. Read-only mode rejects writes with `EROFS`.

## Docker Compose example

The working example lives in [`examples/basic/`](examples/basic/) and keeps all
privileges confined to the scrim service. The application container gets a
plain bind mount of the filtered view.

```bash
cd examples/basic

# 1. Prepare the host-side shared mountpoint (as root)
sudo ../../scripts/setup-mounts.sh

# 2. Start the stack
docker compose up -d

# 3. Use the filtered view
docker compose exec application ls -la /workspace
docker compose exec application cat /workspace/.env   # -> No such file or directory

# 4. Stop and clean up
docker compose down
sudo ../../scripts/cleanup-mounts.sh
```

Only the scrim service receives `SYS_ADMIN`, `/dev/fuse`, and
`apparmor:unconfined`. The application service is health-gated on scrim and
receives no capabilities. Scrim runs as root inside its container because
mounting requires `CAP_SYS_ADMIN`; that container is the deliberately
privileged, single-purpose component of the stack.

Requires a rootful Docker Engine on native Linux. Docker Desktop / VM-based
daemons resolve bind-mount sources in a different mount namespace and are not
supported.

Run the end-to-end verification used during development:

```bash
sudo ./scripts/integration-test.sh
```

## Running scrim directly on the host

Docker is the supported deployment model, but the same binary can run
directly on the host. Because scrim then mounts in the host mount namespace
itself, the propagation setup (`setup-mounts.sh`, `rshared` binds,
`apparmor:unconfined`) is not needed at all -- the application container can
bind-mount the mountpoint directory directly:

```bash
cargo build --locked --release
sudo mkdir -p ./filtered/target
sudo target/release/scrim mount --source "$PWD/project" \
  --mountpoint "$PWD/filtered/target" --config ./config.yaml
docker run -v "$PWD/filtered/target:/workspace" -w /workspace python:3 sleep infinity
```

For a portable binary that runs on any Linux x86_64 host regardless of
installed libraries, build the static musl target:
`cargo build --release --target x86_64-unknown-linux-musl`.

Trade-offs versus the containerized model:

- A host binary is a privileged daemon with the whole host filesystem in
  reach; the scrim container is confined (it only sees `/source` and
  `/filtered`, under Docker's default seccomp profile) and dies with the
  stack. This is the main reason Docker is the primary model.
- Lifecycle is manual: start/stop scrim yourself (or write a systemd unit
  with `Restart=on-failure` around `scrim mount`), and unmount with
  `scrim unmount` before removing directories.
- The fail-safe property is identical: if scrim dies, the mountpoint returns
  I/O errors instead of falling back to the source.

## Known limitations

- Extended attributes (`getxattr`, `setxattr`, `listxattr`) are not forwarded.
- Rename flags such as `RENAME_NOREPLACE` and `RENAME_EXCHANGE` are ignored.
- `hide_excluded_files: false` is treated as `true`.
- Mounting requires root or `CAP_SYS_ADMIN`; unprivileged mounting needs
  `fusermount3`, which is not used by default here.
- The FUSE mount is only accessible to the user that mounted it (root):
  application containers must currently run as root, and host access to the
  filtered mount requires root. `allow_other` is not enabled yet.
- `statfs` reports the source filesystem's statistics, so reported free space
  includes space that is not writable through the filter policy.
- `logging.level` is parsed but not applied yet; logs use the default level.
- After an unclean shutdown, the host mountpoint may linger in
  `Transport endpoint is not connected` state; re-running `setup-mounts.sh`
  clears it before the next start.

## Security model

Scrim prevents accidental access, directory enumeration, guessed filenames,
and rename-based bypasses through the provided mount. The exclusion policy is
enforced in `lookup`, `getattr`, `open`, `read`, `write`, `readdir`, creation,
deletion, and rename, with protection against `..` traversal, absolute-path
escapes, and symlinks that leave the source tree. These paths are covered by
unit tests, but the full integration and security test suite is still planned
for M5; no guarantee should be assumed today.

Scrim does **not** protect against:

- Host root users
- Docker administrators
- Privileged containers
- Access to the Docker socket
- Containers with another route to the original source directory
- Processes that bypass the filtered mount through another mount namespace or
  host path

Application containers must never receive a bind mount of the original source
and must not need `SYS_ADMIN`, `/dev/fuse`, or privileged mode.

## Troubleshooting

**`mount` fails with `EPERM` even with `SYS_ADMIN`.** On Ubuntu, the default
AppArmor profile blocks `mount(2)` inside containers. Use
`security_opt: [apparmor:unconfined]` as in the Compose example.

**The host does not see the FUSE mount.** The host-side `filtered` directory
must be its own shared mount before containers start:

```bash
sudo mkdir -p ./filtered
sudo mount --bind ./filtered ./filtered
sudo mount --make-rshared ./filtered
```

Docker resolves bind-mount source paths on the daemon's host mount namespace,
not the caller's. The directory must exist on the Docker host before the
application container is created.

**The application sees `Transport endpoint is not connected` after the filter
stops.** This is the intended fail-safe behavior: the application must not fall
back to the original source.

**`scrim` refuses to start with `mountpoint is already mounted` after a
previous run.** A dead FUSE mount lingered on the host. Run
`sudo ../../scripts/setup-mounts.sh` again (it clears stale mounts), or
`sudo ../../scripts/cleanup-mounts.sh` to tear everything down.

### Cleanup

If a run is interrupted, unmount before removing directories:

```bash
docker compose down
sudo ../../scripts/cleanup-mounts.sh
```

For a Phase 0 run interrupted before cleanup existed:

```bash
sudo umount -l /tmp/scrim-phase0/filtered/target
sudo umount -l /tmp/scrim-phase0/filtered
rm -rf /tmp/scrim-phase0
```

## Project layout

```
Cargo.toml
Dockerfile            multi-stage build: rust:1-slim-bookworm -> debian:bookworm-slim
src/                  CLI, config, filter, filesystem, mount, health
scripts/
  phase0-feasibility.sh   propagation proof of concept
  setup-mounts.sh         prepare the host-side rshared mountpoint
  cleanup-mounts.sh       unmount FUSE before removing mount directories
  integration-test.sh     end-to-end Compose verification (AC-001..AC-014)
examples/basic/        working Compose example: sample project + config + compose.yaml
tests/                integration and security tests (planned)
config/               example configuration
.github/workflows/    CI
```

## License

No license has been chosen yet.
