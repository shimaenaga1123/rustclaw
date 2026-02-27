FROM --platform=linux/amd64 messense/cargo-zigbuild AS builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /app
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse

RUN rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu

COPY Cargo.toml Cargo.lock ./
COPY src/ src/

ARG TARGETARCH
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target,sharing=locked \
    TARGET=$(case "$TARGETARCH" in \
      amd64) echo "x86_64-unknown-linux-gnu" ;; \
      arm64) echo "aarch64-unknown-linux-gnu" ;; \
    esac) && \
    cargo zigbuild --release --locked --target "$TARGET" && \
    cp target/$TARGET/release/rustclaw /app/rustclaw-bin

FROM gcr.io/distroless/cc-debian13 AS runtime
COPY --from=builder /app/rustclaw-bin /app/rustclaw
WORKDIR /app
ENTRYPOINT ["/app/rustclaw"]