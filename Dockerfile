FROM lukemathwalker/cargo-chef:latest-rust-slim AS chef
RUN apt-get update && apt-get install -y pkg-config g++ mold libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /app
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked

FROM gcr.io/distroless/cc-debian13 AS runtime
COPY --from=builder /lib/x86_64-linux-gnu/libmvec.so.1 /lib/x86_64-linux-gnu/
COPY --from=builder /app/target/release/rustclaw /usr/local/bin/

WORKDIR /app
USER nonroot

ENTRYPOINT ["rustclaw"]