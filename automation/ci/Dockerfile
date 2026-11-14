# CI image for Staminator: toolchain from rust-toolchain.toml, coverage tool, Windows cross-linker.
ARG RUST_VERSION=1.99.1
FROM rust:${RUST_VERSION}
RUN apt-get update \
    && apt-get install -y --no-install-recommends gcc-mingw-w64-x86-64 python3 \
    && rm -rf /var/lib/apt/lists/*
RUN rustup component add rustfmt clippy llvm-tools-preview \
    && rustup target add x86_64-pc-windows-gnu \
    && cargo install cargo-llvm-cov --locked \
    && chmod -R a+rwX /usr/local/cargo /usr/local/rustup
WORKDIR /work
