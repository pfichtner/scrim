# syntax=docker/dockerfile:1

FROM rust:1-slim-bookworm AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --locked --release

# shortcut: scrim runs as root because mount(2) needs CAP_SYS_ADMIN;
# it is the deliberately privileged, single-purpose component of the stack
FROM debian:bookworm-slim
COPY --from=build /build/target/release/scrim /usr/local/bin/scrim
COPY config/config.example.yaml /etc/scrim/config.yaml
HEALTHCHECK --interval=3s --timeout=2s --retries=10 --start-period=5s \
    CMD ["scrim", "healthcheck", "--mountpoint", "/filtered/target"]
ENTRYPOINT ["/usr/local/bin/scrim"]
