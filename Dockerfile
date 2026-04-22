# syntax=docker/dockerfile:1.7

# ---- Builder: Rust toolchain + LLVM 21 dev libraries ----
FROM rust:1.95-bookworm AS builder

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates gnupg wget && \
    wget -qO- https://apt.llvm.org/llvm-snapshot.gpg.key \
        | gpg --dearmor -o /usr/share/keyrings/apt-llvm.gpg && \
    echo "deb [signed-by=/usr/share/keyrings/apt-llvm.gpg] https://apt.llvm.org/bookworm/ llvm-toolchain-bookworm-21 main" \
        > /etc/apt/sources.list.d/apt-llvm.list && \
    apt-get update && \
    apt-get install -y --no-install-recommends \
        llvm-21-dev libpolly-21-dev zlib1g-dev libzstd-dev && \
    rm -rf /var/lib/apt/lists/*

ENV LLVM_SYS_210_PREFIX=/usr/lib/llvm-21

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --locked


# ---- Runtime: libLLVM-21 runtime + cc (for linking user programs) ----
FROM debian:bookworm-slim AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates gnupg wget && \
    wget -qO- https://apt.llvm.org/llvm-snapshot.gpg.key \
        | gpg --dearmor -o /usr/share/keyrings/apt-llvm.gpg && \
    echo "deb [signed-by=/usr/share/keyrings/apt-llvm.gpg] https://apt.llvm.org/bookworm/ llvm-toolchain-bookworm-21 main" \
        > /etc/apt/sources.list.d/apt-llvm.list && \
    apt-get update && \
    apt-get install -y --no-install-recommends \
        libllvm21 gcc libc6-dev && \
    apt-get purge -y --auto-remove gnupg wget && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/beer /usr/local/bin/beer

WORKDIR /work
ENTRYPOINT ["beer"]
CMD ["--help"]
