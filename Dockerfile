FROM rust:1.82.0-slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev make

WORKDIR /app

COPY . .
RUN --mount=type=cache,target=/app/target \
    --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release && cp target/release/mapdiffbot2 target/release/icondiffbot2 .

FROM debian:bookworm-20241016-slim AS base

RUN apt-get update && apt-get install -y libssl3
USER 1000
WORKDIR /app

FROM base AS mapdiffbot2
COPY --from=builder /app/mapdiffbot2 /app/mapdiffbot2

ENTRYPOINT /app/mapdiffbot2

FROM base AS icondiffbot2
COPY --from=builder /app/icondiffbot2 /app/icondiffbot2

ENTRYPOINT /app/icondiffbot2
