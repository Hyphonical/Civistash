# ── Stage 1: Build ────────────────────────────────────────
FROM rust:1.88-slim AS builder

WORKDIR /build

RUN apt-get update && apt-get install -y \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Pre-fetch dependencies for better layer caching
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src \
    && echo "fn main() {}" > src/main.rs \
    && cargo build --release \
    && rm -rf src target/release/deps/civistash*

# Compile the real source
COPY src ./src
RUN touch src/main.rs \
    && cargo build --release

# ── Stage 2: Runtime ──────────────────────────────────────
FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /build/target/release/civistash /usr/local/bin/civistash
COPY LICENSE /LICENSE

USER nonroot
WORKDIR /stash
VOLUME ["/stash"]

ENV CIVITAI_TOKEN=""
ENV RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/civistash"]
