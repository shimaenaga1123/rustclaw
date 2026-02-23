FROM lukemathwalker/cargo-chef:latest-rust-slim AS chef
RUN apt-get update && apt-get install -y pkg-config g++ mold libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /app
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
ARG BUILDKIT_INLINE_CACHE=1

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ARG TARGETARCH
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked

FROM gcr.io/distroless/cc-debian13 AS runtime
ARG TARGETARCH
COPY --from=builder /app/target/release/rustclaw /app/
WORKDIR /app
ENTRYPOINT ["rustclaw"]