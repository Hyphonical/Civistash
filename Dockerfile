# ── Stage 1: Build ────────────────────────────────────────
# konst 0.4.3 and redb 3.1.3 (transitive deps via hf-hub) require
# rustc >= 1.89.0. Keep this in sync with Cargo.toml rust-version.
FROM rust:1.89-slim AS builder

WORKDIR /build

# aws-lc-sys (pulled in by reqwest → rustls → aws-lc-rs)
# compiles AWS-LC from source and needs cmake + a C toolchain.
# `pkg-config` is used by flate2 to detect zlib at build time;
# if it's absent, flate2 falls back to the bundled `miniz_oxide`
# crate (pure Rust, no C dependency), so it's harmless but
# included for a zero-warning build log. `ca-certificates` keeps
# `cargo build` from failing on HTTPS crate downloads in slim
# images that ship without certs.
RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake \
    gcc \
    g++ \
    make \
    pkg-config \
    ca-certificates \
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
FROM gcr.io/distroless/cc-debian13:nonroot

COPY --from=builder /build/target/release/civistash /usr/local/bin/civistash
COPY LICENSE /LICENSE

USER nonroot
WORKDIR /stash
RUN mkdir -p /stash && chown nonroot:nonroot /stash
VOLUME ["/stash"]

# No ENV placeholders for tokens — supply them at runtime via
# `docker run -e CIVITAI_TOKEN=...` or a docker-compose .env file.
# The binary reads CIVITAI_TOKEN and HUGGINGFACE_TOKEN directly
# (clap `env = "..."` derive), so no forwarding is needed.
ENV RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/civistash"]
