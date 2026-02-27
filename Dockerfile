FROM --platform=$BUILDPLATFORM rust:slim AS builder
RUN dpkg --add-architecture arm64 && \
    apt-get update && apt-get install -y \
    pkg-config \
    g++ libssl-dev \
    g++-aarch64-linux-gnu libc6-dev-arm64-cross libssl-dev:arm64 \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse

RUN rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu

ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++ \
    PKG_CONFIG_PATH_aarch64_unknown_linux_gnu=/usr/lib/aarch64-linux-gnu/pkgconfig \
    PKG_CONFIG_ALLOW_CROSS=1

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
    cargo build --release --locked --target "$TARGET" && \
    cp target/$TARGET/release/rustclaw /app/rustclaw-bin

FROM gcr.io/distroless/cc-debian13 AS runtime
COPY --from=builder /app/rustclaw-bin /app/rustclaw
WORKDIR /app
ENTRYPOINT ["/app/rustclaw"]