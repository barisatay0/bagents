FROM rust:bookworm AS builder

WORKDIR /usr/src/bagents

COPY . .

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

RUN cargo build --release

FROM debian:bookworm-slim

ARG PROJECT_LANG="rust"

RUN apt-get update && apt-get install -y \
    ca-certificates \
    git \
    ssh \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

RUN if [ "$PROJECT_LANG" = "node" ]; then \
        echo "Node.js Environment Building..." && \
        curl -fsSL https://deb.nodesource.com/setup_20.x | bash - && \
        apt-get install -y nodejs && \
        rm -rf /var/lib/apt/lists/*; \
    elif [ "$PROJECT_LANG" = "python" ]; then \
        echo "Python Environment Building..." && \
        apt-get update && apt-get install -y python3 python3-pip && \
        rm -rf /var/lib/apt/lists/*; \
    elif [ "$PROJECT_LANG" = "rust" ]; then \
        echo "Rust Environment Building..." && \
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y; \
    fi

ENV PATH="/root/.cargo/bin:${PATH}"

RUN git config --global --add safe.directory /workspace

WORKDIR /app

COPY --from=builder /usr/src/bagents/target/release/bagents /usr/local/bin/bagents

COPY ./config /app/config

ENV WORKSPACE_DIR=/workspace

CMD ["bagents"]
