# Build the vcsstar server and CLI. Two stages: a Rust builder and a slim
# runtime that carries git (for the VCS layer) and CA certificates.
FROM rust:1.98-bookworm AS builder
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends libpq-dev pkg-config && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations
RUN cargo build --release --locked --workspace \
    && strip target/release/vcsstard target/release/vcsstar

# The web UI: SolidJS built with Vite into static files.
FROM node:22-bookworm-slim AS ui
WORKDIR /ui
COPY ui/package.json ui/package-lock.json ./
RUN npm ci --ignore-scripts && npm rebuild esbuild
COPY ui/ ./
RUN npm run build

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libpq5 git openssh-client \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /var/lib/vcsstar --create-home vcsstar
COPY --from=builder /src/target/release/vcsstard /usr/local/bin/vcsstard
COPY --from=builder /src/target/release/vcsstar /usr/local/bin/vcsstar
COPY --from=ui /ui/dist /usr/local/share/vcsstar/ui
ENV VCSSTAR__UI_DIR=/usr/local/share/vcsstar/ui
USER 10001
WORKDIR /var/lib/vcsstar
EXPOSE 8080 4988
ENTRYPOINT ["/usr/local/bin/vcsstard"]
CMD ["serve", "--roles", "api"]

# CLI-only image for CI jobs: `vcsstar` on PATH and nothing else.
FROM debian:bookworm-slim AS cli
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /home/vcsstar --create-home vcsstar
COPY --from=builder /src/target/release/vcsstar /usr/local/bin/vcsstar
USER 10001
ENTRYPOINT ["/usr/local/bin/vcsstar"]
