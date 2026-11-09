# Build the vscstar server and CLI. Two stages: a Rust builder and a slim
# runtime that carries git (for the VCS layer) and CA certificates.
FROM rust:1.98-bookworm AS builder
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends libpq-dev pkg-config && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations
RUN cargo build --release --locked --workspace \
    && strip target/release/vscstard target/release/vscstar

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libpq5 git openssh-client \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /var/lib/vscstar --create-home vscstar
COPY --from=builder /src/target/release/vscstard /usr/local/bin/vscstard
COPY --from=builder /src/target/release/vscstar /usr/local/bin/vscstar
USER 10001
WORKDIR /var/lib/vscstar
EXPOSE 8080 4988
ENTRYPOINT ["/usr/local/bin/vscstard"]
CMD ["serve", "--roles", "api"]

# CLI-only image for CI jobs: `vscstar` on PATH and nothing else.
FROM debian:bookworm-slim AS cli
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /home/vscstar --create-home vscstar
COPY --from=builder /src/target/release/vscstar /usr/local/bin/vscstar
USER 10001
ENTRYPOINT ["/usr/local/bin/vscstar"]
