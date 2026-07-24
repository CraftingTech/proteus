# syntax=docker/dockerfile:1.7

# --- UI (Dioxus WASM) ---------------------------------------------------
FROM rust:1.91-bookworm AS ui
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config binaryen \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add wasm32-unknown-unknown \
    && cargo install dioxus-cli --version 0.7.9 --locked

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN dx build -p proteus-ui --platform web --release \
    && mkdir -p crates/proteus-ui/dist \
    && cp -a target/dx/proteus-ui/release/web/public/. crates/proteus-ui/dist/

# --- Controller ---------------------------------------------------------
FROM rust:1.91-bookworm AS builder
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY --from=ui /src/crates/proteus-ui/dist ./crates/proteus-ui/dist

RUN cargo build --release -p proteus-controller

# --- Runtime ------------------------------------------------------------
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /src/target/release/proteus-controller /usr/local/bin/proteus-controller
USER nonroot:nonroot
EXPOSE 8080
ENV PROTEUS_API_ADDR=0.0.0.0:8080
ENTRYPOINT ["/usr/local/bin/proteus-controller"]
