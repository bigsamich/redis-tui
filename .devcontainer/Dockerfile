FROM rust:latest

RUN apt-get update && apt-get install -y \
    musl-tools \
    pkg-config \
    git \
    redis-tools \
    && rm -rf /var/lib/apt/lists/*

RUN rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl \
    && rustup component add rust-analyzer clippy rustfmt

WORKDIR /workspace
