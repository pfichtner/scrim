# Scrim

Writable, filtered bind mounts for Docker containers. Scrim exposes a host
directory to application containers through a FUSE filesystem that hides
configured files and directories such as `.env`, private keys, and `.git/`,
while keeping normal read/write behavior and write-through to the original
directory.

Scrim is the project name. The Cargo package and binary still use their
original names, `docker-fuse-filter` and `fuse-filter`.

> **Status: pre-release, M1 complete.**
> The FUSE passthrough filesystem and CLI mount lifecycle work on the host, and
> exclusion patterns are enforced on filesystem operations. Docker image/Compose
> integration and the full security test suite are still under implementation.
> Do not rely on Scrim to protect real secrets yet.

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
| Dockerfile and Docker Compose example | Planned (M4) |
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
fuse-filter mount \
  --source /source \
  --mountpoint /filtered/target \
  --config /etc/fuse-filter/config.yaml

# Check filesystem health
fuse-filter healthcheck --mountpoint /filtered/target

# Validate configuration
fuse-filter validate --config /etc/fuse-filter/config.yaml

# Display active configuration
fuse-filter config show --config /etc/fuse-filter/config.yaml

# Unmount filesystem
fuse-filter unmount --mountpoint /filtered/target
```

Exit codes are `0` success, `1` general error, `2` invalid configuration,
`3` source unavailable, `4` mount failure, and `5` filesystem unhealthy.
`--config` defaults to `/etc/fuse-filter/config.yaml` for `mount`, `validate`,
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

## Known limitations

- Extended attributes (`getxattr`, `setxattr`, `listxattr`) are not forwarded.
- Rename flags such as `RENAME_NOREPLACE` and `RENAME_EXCHANGE` are ignored.
- `hide_excluded_files: false` is treated as `true`.
- Mounting requires root or `CAP_SYS_ADMIN`; unprivileged mounting needs
  `fusermount3`, which is not used by default here.
- `statfs` reports the source filesystem's statistics, so reported free space
  includes space that is not writable through the filter policy.
- `logging.level` is parsed but not applied yet; logs use the default level.

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

## Docker Compose (planned)

The target Compose layout keeps privileges confined to the filter container:

```yaml
services:
  fuse-filter:
    cap_add:
      - SYS_ADMIN
    devices:
      - /dev/fuse:/dev/fuse
    security_opt:
      - apparmor:unconfined
    volumes:
      - type: bind
        source: ./project
        target: /source
      - type: bind
        source: ./filtered
        target: /filtered
        bind:
          propagation: rshared

  application:
    depends_on:
      fuse-filter:
        condition: service_healthy
    volumes:
      - type: bind
        source: ./filtered/target
        target: /workspace
        bind:
          propagation: rslave
```

The Dockerfile, Compose file, health check, and host mount setup/cleanup
scripts are not present yet; they arrive in M4.

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

### Cleanup

If a Phase 0 run is interrupted, unmount before removing directories:

```bash
sudo umount -l /tmp/docker-fuse-filter-phase0/filtered/target
sudo umount -l /tmp/docker-fuse-filter-phase0/filtered
rm -rf /tmp/docker-fuse-filter-phase0
```

## Project layout

```
Cargo.toml
src/                  CLI, config, filter, filesystem, mount, health
scripts/
  phase0-feasibility.sh
tests/                integration and security tests (planned)
examples/             example projects (planned)
config/               example configuration
.github/workflows/    CI
```

## License

No license has been chosen yet.
