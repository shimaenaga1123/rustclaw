FROM rust:slim AS builder
RUN apt-get update && apt-get install -y pkg-config g++ mold libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /app
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse

COPY .cargo/ .cargo/
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build --release --locked && \
    cp target/release/rustclaw /app/rustclaw-bin

FROM gcr.io/distroless/cc-debian13 AS runtime
COPY --from=builder /app/rustclaw-bin /app/rustclaw
WORKDIR /app
ENTRYPOINT ["/app/rustclaw"]