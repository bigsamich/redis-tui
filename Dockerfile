# Build stage - use Alpine + musl for fully static binary
FROM rust:latest AS builder

RUN apt-get update && apt-get install -y musl-tools && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl

WORKDIR /app

# Detect architecture and set target
ARG TARGETARCH
RUN case "$TARGETARCH" in \
      arm64) echo "aarch64-unknown-linux-musl" > /tmp/rust-target ;; \
      *)     echo "x86_64-unknown-linux-musl" > /tmp/rust-target ;; \
    esac

# Cache dependencies by copying manifests first
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release --target $(cat /tmp/rust-target) \
    && rm -rf src

# Build the actual application
COPY src/ src/
RUN touch src/main.rs \
    && cargo build --release --target $(cat /tmp/rust-target) \
    && cp target/$(cat /tmp/rust-target)/release/redis-tui /redis-tui

# Runtime stage - scratch since the binary is fully static
FROM alpine:latest

COPY --from=builder /redis-tui /usr/local/bin/redis-tui

CMD ["redis-tui"]
