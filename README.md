# Scrim

**A filtered, writable filesystem for sandboxed applications and AI
coding agents.**

Scrim exposes a host directory through a FUSE filesystem that enforces
configurable file and directory exclusions. Applications can read,
create, modify, and delete permitted files while excluded paths remain
inaccessible through the filtered mount.

Use Scrim to:

-   Give AI coding agents access to source code without exposing
    selected credentials and private configuration files.
-   Run development tools against a real project directory with a
    restricted filesystem view.
-   Expose selected project files to build systems and CI/CD jobs.
-   Provide writable project access without exposing every file in the
    source tree.

Scrim supports Docker-based deployment and can also run directly on a
Linux host.

> \[!WARNING\] **Pre-release software --- do not rely on Scrim to
> protect real secrets yet.**
>
> The FUSE passthrough filesystem, exclusion enforcement, write-through
> behavior, and Docker image/Compose integration (including mount
> propagation) work end to end. The automated security test suite and
> failure watchdog are still under implementation. Treat Scrim as
> experimental until those are complete and the security behavior has
> been thoroughly reviewed.

## AI coding agents

AI coding agents often need broad access to a project directory. They
may need to inspect source code, modify files, create new files, run
tests, and execute development tools. They generally do not need access
to every credential or private configuration file stored alongside the
source code.

Scrim can provide the agent with a filtered view of the project:

``` text
Original project/              Filtered view (/workspace)
├── src/                       ├── src/
├── tests/                     ├── tests/
├── package.json               ├── package.json
├── README.md                  └── README.md
├── .env
├── .env.production
├── .git/
├── secrets/
│   └── deploy.key
└── private.pem
```

For example, a policy might exclude environment files, private keys, and
a secrets directory:

``` yaml
filter:
  patterns:
    - ".env"
    - ".env.*"
    - "*.key"
    - "*.pem"
    - "**/secrets/**"
```

The agent receives access to the filtered mount, not the original
project directory. Attempts to access excluded paths through that mount
are intended to fail with `ENOENT`, including attempts to open a known
filename directly.

The filtered view remains writable. Changes to permitted files are
written through immediately to the original directory.

### Important considerations for coding agents

-   **Do not provide another route to the source.** The agent must not
    also receive the original project as a bind mount, a
    parent-directory mount, or another filesystem path that exposes
    excluded files.
-   **Review the policy for your project.** Excluding `.git/` prevents
    ordinary Git commands from accessing that working tree. If the agent
    needs Git, consider whether `.git/` can remain visible under your
    threat model.
-   **Control other channels separately.** Scrim does not remove secrets
    already present in environment variables, inherited file
    descriptors, process memory, accessible services, or other mounts.
-   **Use a separate sandbox.** Scrim filters filesystem access through
    one mount; it does not provide process isolation, network
    restrictions, resource limits, or protection against every host
    interface.

The coding-agent example below is illustrative. It is not a security
certification or a verified integration with any particular agent.

## Why

Applications frequently need access to a writable project directory, but
they do not necessarily need access to everything inside it.

This is especially relevant for AI coding agents and third-party
development tools. They may need broad access to source code to perform
useful work, while credentials, private keys, and other sensitive files
should remain outside their view.

A conventional bind mount exposes the entire directory. A read-only
mount prevents modifications but does not provide selective visibility.
Hiding names in a directory listing alone is also insufficient: a
process may try to open a file by its exact path.

Scrim provides an intermediate option: a writable filesystem view that
enforces configured exclusions across filesystem operations.

## How it works

``` text
                         HOST
                           |
                    /project (source)
                           |
                       bind mount
                           |
                           v
             +-----------------------------+
             | SCRIM FILTER CONTAINER      |
             |                             |
             |   /source                   |
             |      |                      |
             |      v                      |
             |   FUSE FILESYSTEM           |
             |      |                      |
             |      v                      |
             |   /filtered/target          |
             +-----------------------------+
                           |
                    mount propagation
                           |
                           v
             +-----------------------------+
             | APPLICATION CONTAINER       |
             |                             |
             |   /workspace                |
             |   (filtered view)           |
             +-----------------------------+
```

The source directory is made available to Scrim. Scrim mounts a FUSE
passthrough filesystem at the filtered target. With the Docker
deployment, mount propagation makes that FUSE mount visible on the host,
where the application container bind-mounts the filtered view.

Only the Scrim container receives `SYS_ADMIN` and `/dev/fuse`. The
application container receives an ordinary bind mount and does not need
those privileges.

Filtering directory listings alone is not enough. Scrim is designed to
enforce exclusions on direct path lookup and file operations too, so
excluded paths fail with `ENOENT` through the filtered mount.

## Current status

**Status: pre-release, M4 complete.**

  -----------------------------------------------------------------------
  Area                                Status
  ----------------------------------- -----------------------------------
  Mount propagation (FUSE → host →    Verified on native Linux Docker
  application)                        

  FUSE passthrough filesystem         Implemented (M1), manually verified

  Exclusion enforcement in filesystem Implemented (M2), integration tests
  operations                          pending

  Write-through read/write support    Implemented (M3), integration tests
                                      pending

  Docker image and Docker Compose     Implemented (M4), end-to-end tested
  example                             on native Linux

  Automated security tests, failure   Planned (M5)
  watchdog, benchmarks                
  -----------------------------------------------------------------------

See [`implementation-state.md`](implementation-state.md) for
implementation details.

Do not treat the current implementation as a dependable security
boundary for production credentials. Unit tests and development checks
are not a substitute for the planned integration/security suite and
independent review.

## Deployment models

Scrim has a filesystem core and more than one way to expose its filtered
mount to an application.

### Docker deployment (primary documented deployment)

The Docker Compose example runs Scrim in a dedicated FUSE container and
exposes the propagated filtered mount to an application container.

Advantages:

-   The special mount privileges are confined to the Scrim service.
-   The application receives a normal bind mount.
-   The Scrim and application lifecycles can be managed together with
    Docker Compose.

Requirements:

-   Native Linux Docker Engine.
-   Rootful Docker and Docker Compose.
-   Host-side mount preparation with `sudo`.

Docker Desktop and other VM-based Docker daemons are not supported
because their bind-mount sources are resolved in a different mount
namespace.

See [Docker Compose example](#docker-compose-example).

### Host deployment

The same Scrim binary can run directly on a Linux host. In this model,
Scrim mounts in the host mount namespace, and an application can
bind-mount the resulting filtered directory.

This can be useful for:

-   Local coding agents.
-   IDEs and development tools.
-   Docker containers.
-   Build processes and other applications that can consume a directory
    mount.

Host deployment avoids the Docker mount-propagation setup, but Scrim
itself runs with host-level privileges. This is a significant trade-off;
see [Running Scrim directly on the
host](#running-scrim-directly-on-the-host).

## Prerequisites

-   Linux with FUSE support (`/dev/fuse`).
-   For the Docker deployment: a rootful Docker Engine and Docker
    Compose.
-   Rust stable toolchain for building from source.
-   `sudo` access on the Docker host for mount preparation.

The Docker deployment requires native Linux Docker. Docker Desktop /
VM-based daemons are not supported.

## Build from source

``` bash
cargo build --locked
```

Run the checks used by CI:

``` bash
cargo fmt --all -- --check
cargo build --locked --all-targets
cargo clippy --locked --all-targets
cargo test --locked
```

## Verify mount propagation (Phase 0)

The feasibility script proves the mount-propagation architecture end to
end using `bindfs` as a minimal FUSE stand-in. It creates a temporary
source directory, mounts it through a container, confirms that the mount
is visible on the host, starts an application container without extra
capabilities, checks write-through, and confirms that killing the FUSE
container does not expose the source.

Run on the native Linux Docker host as root:

``` bash
sudo ./scripts/phase0-feasibility.sh
```

The script cleans up its containers, mounts, and temporary directories
when it exits.

## CLI

The intended interface:

``` bash
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

# Unmount the filesystem
scrim unmount --mountpoint /filtered/target
```

Exit codes:

  Code   Meaning
  ------ -----------------------
  `0`    Success
  `1`    General error
  `2`    Invalid configuration
  `3`    Source unavailable
  `4`    Mount failure
  `5`    Filesystem unhealthy

`--config` defaults to `/etc/scrim/config.yaml` for `mount`, `validate`,
and `config show`.

## Configuration

Example configuration:

``` yaml
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

No exclusions are applied by default; every pattern must be configured
explicitly.

Defaults for omitted fields:

  Setting                  Default
  ------------------------ ---------
  `read_only`              `false`
  `allow_symlinks`         `false`
  `preserve_permissions`   `true`
  `case_sensitive`         `true`
  `hide_excluded_files`    `true`
  `logging.level`          `info`

### Pattern semantics

Patterns are matched against paths relative to the source root.

-   Bare names match at any depth. For example, `.env` also hides
    `config/.env`.
-   A trailing `/` marks a directory pattern and hides the directory
    itself and everything below it.
-   `*` does not cross a path separator.
-   `**` can cross path separators.
-   Negation (`!pattern`) is not implemented.
-   Case-insensitive matching is configurable.
-   `hide_excluded_files: false` is currently treated as `true`.

  Pattern           Intended meaning
  ----------------- --------------------------------------------------
  `.env`            Hide files named `.env`
  `.env.*`          Hide `.env.local`, `.env.production`, etc.
  `*.pem`           Hide PEM files
  `*.key`           Hide key files
  `.git/`           Hide directories named `.git` and their contents
  `**/secrets/**`   Hide contents below any `secrets` directory

Patterns are explicit policy, not a guarantee that every sensitive file
in a project has been identified. Review the source tree and policy for
each use case.

## Write-through behavior

Scrim is a passthrough filesystem, not a copy-on-write or staging layer.

Reads and writes operate directly on the source directory. A change made
through the filtered mount is immediately visible in the original
directory; no synchronization process is involved.

When `read_only: true`, writes are rejected with `EROFS`.

## Example: expose a project to a coding agent

This example illustrates how an application can be given the filtered
view instead of the original project. It assumes Scrim is already
mounted and that the application container has no other route to the
source directory.

Suppose the project contains:

``` text
project/
├── src/
├── tests/
├── package.json
├── .env
└── secrets/
    └── api.key
```

A configuration could exclude the environment file and secrets
directory:

``` yaml
filesystem:
  source: /source
  mountpoint: /filtered/target
  read_only: false

filter:
  patterns:
    - ".env"
    - "**/secrets/**"
```

For a host-side Scrim mount, an application container could consume the
filtered view like this:

``` bash
docker run --rm \
  -v "$PWD/filtered/target:/workspace" \
  -w /workspace \
  my-coding-agent
```

The agent can work with permitted files in `/workspace`. Changes to
those files are written through to the original project.

**This example is illustrative, not a verified integration with a
specific coding-agent product.** Adapt the container command to the
agent's actual runtime and sandbox requirements.

Before using this architecture for sensitive data, verify all of the
following:

1.  The agent receives only the filtered mount, not the original source
    directory or a parent directory containing it.
2.  The agent cannot access the host Docker socket or another privileged
    control interface.
3.  Secrets are not also supplied through environment variables,
    command-line arguments, inherited file descriptors, mounted
    credentials, or accessible processes.
4.  Network access and other exfiltration routes are controlled
    separately where necessary.
5.  The exact Scrim build and configuration have passed an appropriate
    security test suite.

## Docker Compose example

The working example lives in [`examples/basic/`](examples/basic/) and
keeps the mount privileges in the Scrim service. The application
container gets a plain bind mount of the filtered view.

``` bash
cd examples/basic

# 1. Prepare the host-side shared mount (as root)
sudo ../../scripts/setup-mounts.sh

# 2. Start the stack
docker compose up -d

# 3. Use the filtered view
docker compose exec application ls -la /workspace
docker compose exec application cat /workspace/.env
# Expected: No such file or directory

# 4. Stop and clean up
docker compose down
sudo ../../scripts/cleanup-mounts.sh
```

Only the Scrim service receives `SYS_ADMIN`, `/dev/fuse`, and
`apparmor:unconfined`. The application service is health-gated on Scrim
and receives no capabilities.

Scrim runs as root inside its container because mounting requires
`CAP_SYS_ADMIN`. That container is the deliberately privileged,
single-purpose component of the stack.

The Docker deployment requires a rootful Docker Engine on native Linux.
Docker Desktop / VM-based daemons are not supported.

Run the end-to-end verification used during development:

``` bash
sudo ./scripts/integration-test.sh
```

## Running Scrim directly on the host

Docker is the primary documented deployment model, but the same binary
can run directly on the host.

Because Scrim then mounts in the host mount namespace itself, the
propagation setup (`setup-mounts.sh`, `rshared` binds, and
`apparmor:unconfined`) is not needed. An application container can
bind-mount the mountpoint directory directly.

Example:

``` bash
cargo build --locked --release

sudo mkdir -p ./filtered/target

sudo target/release/scrim mount \
  --source "$PWD/project" \
  --mountpoint "$PWD/filtered/target" \
  --config ./config.yaml

docker run --rm \
  -v "$PWD/filtered/target:/workspace" \
  -w /workspace \
  python:3 sleep infinity
```

For a portable binary targeting Linux x86_64, build the static musl
target:

``` bash
cargo build --release --target x86_64-unknown-linux-musl
```

### Host deployment trade-offs

-   **Privilege scope:** A host binary is a privileged process with
    access to the host mount namespace and the filesystem permissions
    available to it. The Scrim container is more confined: it sees only
    `/source` and `/filtered`, subject to the container configuration
    and Docker's default seccomp profile.
-   **Lifecycle:** Host deployment requires you to start and stop Scrim
    yourself, or manage it with something such as a systemd unit using
    `Restart=on-failure`.
-   **Cleanup:** Unmount with `scrim unmount` before removing mount
    directories.
-   **Failure behavior:** The intended fail-safe behavior is the same:
    if Scrim dies, access through the FUSE mount should fail with I/O
    errors rather than falling back to the source directory. This
    behavior must be verified for the exact deployment and failure mode.

## Known limitations

-   Extended attributes (`getxattr`, `setxattr`, `listxattr`) are not
    forwarded.
-   Rename flags such as `RENAME_NOREPLACE` and `RENAME_EXCHANGE` are
    ignored.
-   `hide_excluded_files: false` is currently treated as `true`.
-   Mounting requires root or `CAP_SYS_ADMIN`; unprivileged mounting
    through `fusermount3` is not used by default.
-   The FUSE mount is only accessible to the user that mounted it
    (root). Application containers must currently run as root, and host
    access to the filtered mount requires root. `allow_other` is not
    enabled yet.
-   `statfs` reports the source filesystem's statistics, so reported
    free space includes space that is not writable through the filter
    policy.
-   `logging.level` is parsed but not applied yet; logs use the default
    level.
-   After an unclean shutdown, the host mountpoint may remain in a
    `Transport endpoint is not connected` state. Re-running
    `setup-mounts.sh` clears it before the next start.

## Security model

Scrim is a filesystem filtering component, not a complete application
sandbox.

The intended security property is that excluded paths cannot be accessed
through the filtered filesystem, even when a process knows their names.
The exclusion policy is enforced in `lookup`, `getattr`, `readdir`,
`open`, `read`, `write`, `create`, `mkdir`, `unlink`, `rmdir`, `rename`,
`link`, and `symlink`.

The implementation is designed to address `..` traversal, absolute-path
escapes, and symlinks that leave the source tree. These paths have
unit-test coverage, but the full integration and security test suite is
still planned for M5. Do not assume the implementation provides a
security guarantee today.

### What Scrim does not protect against

Scrim does not protect against:

-   Host root users.
-   Docker administrators.
-   Privileged containers.
-   Access to the Docker socket.
-   Containers or processes with another route to the original source
    directory.
-   Processes that bypass the filtered mount through another mount
    namespace or host path.
-   Secrets supplied through environment variables, command-line
    arguments, inherited file descriptors, process memory, or other
    accessible services.
-   Network-based exfiltration or other channels outside the filtered
    filesystem.

Application containers must never receive a bind mount of the original
source directory. They must not need `SYS_ADMIN`, `/dev/fuse`, or
privileged mode.

A deployment that uses Scrim to limit an AI coding agent should also
apply appropriate process/container isolation, network policy,
credential handling, and resource limits. Scrim addresses filesystem
visibility through its mount; it does not replace those controls.

## Troubleshooting

**`mount` fails with `EPERM` even with `SYS_ADMIN`.**

On Ubuntu, the default AppArmor profile can block `mount(2)` inside
containers. Use `security_opt: [apparmor:unconfined]` as in the Compose
example. This setting belongs on the Scrim service, not the application
service.

**The host does not see the FUSE mount.**

The host-side `filtered` directory must be its own shared mount before
containers start:

``` bash
sudo mkdir -p ./filtered
sudo mount --bind ./filtered ./filtered
sudo mount --make-rshared ./filtered
```

Docker resolves bind-mount source paths in the daemon host's mount
namespace, not the caller's. The directory must exist on the Docker host
before the application container is created.

**The application sees `Transport endpoint is not connected` after the
filter stops.**

This is the intended fail-closed behavior: the application must not fall
back to the original source. Restart Scrim and verify the mount before
allowing the application to continue.

**`scrim` refuses to start with `mountpoint is already mounted` after a
previous run.**

A dead FUSE mount may have lingered on the host. Run
`sudo ../../scripts/setup-mounts.sh` again to clear stale mounts, or run
`sudo ../../scripts/cleanup-mounts.sh` to tear everything down.

### Cleanup

If a run is interrupted, unmount before removing directories:

``` bash
docker compose down
sudo ../../scripts/cleanup-mounts.sh
```

For a Phase 0 run interrupted before cleanup existed:

``` bash
sudo umount -l /tmp/scrim-phase0/filtered/target
sudo umount -l /tmp/scrim-phase0/filtered
rm -rf /tmp/scrim-phase0
```

## Project layout

``` text
Cargo.toml
Dockerfile                 multi-stage build: rust:1-slim-bookworm -> debian:bookworm-slim
src/                       CLI, config, filter, filesystem, mount, health
scripts/
  phase0-feasibility.sh    propagation proof of concept
  setup-mounts.sh          prepare the host-side rshared mountpoint
  cleanup-mounts.sh        unmount FUSE before removing mount directories
  integration-test.sh      end-to-end Compose verification (AC-001..AC-014)
examples/basic/            working Compose example: sample project + config + compose.yaml
tests/                     integration and security tests (planned)
config/                    example configuration
.github/workflows/         CI
```

## License

No license has been chosen yet.

## Testing

- Unit tests: `cargo test --locked` (or `cargo test` without a lockfile in dev).
- FUSE integration/security tests require `/dev/fuse` and root on a native Linux host: `sudo cargo test --test filesystem -- --ignored` (uses the in-repo `scrim` crate via a spawned FUSE mount).
- Docker end-to-end: `sudo ./scripts/integration-test.sh` (rootful Docker, native Linux; covers AC-001..AC-014).
- CI runs `cargo fmt --check`, `cargo build --locked --all-targets`, `cargo clippy --locked --all-targets`, `cargo test --locked`.
