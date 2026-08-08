# syntax=docker/dockerfile:1.7

ARG RUST_VERSION=1.91

# sccache is inert in the vendor-free path. BoringCache's managed builder
# supplies its session-only configuration when Docker tool caching is enabled.
FROM rust:${RUST_VERSION}-bookworm AS rust-cache
ARG SCCACHE_VERSION=0.16.0
ARG SCCACHE_SHA256_AMD64=aec995a83ad3dff3d14b6314e08858b7b73d35ca85a5bcf3d3a9ec07dee35588
ARG SCCACHE_SHA256_ARM64=f73a5c39f96bb6ebb89cc7915cf182260d4cbf30765322c5e793d0fe8bd80784

RUN set -eux; \
    case "$(uname -m)" in \
      x86_64) sccache_target=x86_64-unknown-linux-musl; sccache_sha256="$SCCACHE_SHA256_AMD64" ;; \
      aarch64) sccache_target=aarch64-unknown-linux-musl; sccache_sha256="$SCCACHE_SHA256_ARM64" ;; \
      *) echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;; \
    esac; \
    sccache_archive="sccache-v${SCCACHE_VERSION}-${sccache_target}.tar.gz"; \
    curl -fsSL "https://github.com/mozilla/sccache/releases/download/v${SCCACHE_VERSION}/${sccache_archive}" -o "/tmp/${sccache_archive}"; \
    printf '%s  %s\n' "$sccache_sha256" "/tmp/${sccache_archive}" | sha256sum -c -; \
    tar -xzf "/tmp/${sccache_archive}" -C /tmp; \
    install -m 0755 "/tmp/sccache-v${SCCACHE_VERSION}-${sccache_target}/sccache" /usr/local/bin/sccache; \
    rm -rf "/tmp/${sccache_archive}" "/tmp/sccache-v${SCCACHE_VERSION}-${sccache_target}"; \
    sccache --version

# --- UI (Dioxus WASM) ---------------------------------------------------
FROM rust-cache AS ui
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config binaryen \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add wasm32-unknown-unknown \
    && cargo install dioxus-cli --version 0.7.9 --locked \
    && if [ -n "${RUSTC_WRAPPER:-}" ]; then sccache --show-stats; fi

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
ARG BUILDARCH
# Dioxus supplies its own workspace wrapper, so only this command clears the
# BoringCache-injected compiler wrapper. The target mount preserves its output.
RUN --mount=type=cache,target=/src/target,id=proteus-target-ui-${BUILDARCH},sharing=locked \
    RUSTC_WRAPPER= dx build -p proteus-ui --platform web --release \
    && mkdir -p crates/proteus-ui/dist \
    && cp -a target/dx/proteus-ui/release/web/public/. crates/proteus-ui/dist/

# --- Controller ---------------------------------------------------------
FROM rust-cache AS builder
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY --from=ui /src/crates/proteus-ui/dist ./crates/proteus-ui/dist

ARG TARGETARCH
RUN --mount=type=cache,target=/src/target,id=proteus-target-controller-${TARGETARCH},sharing=locked \
    cargo build --release -p proteus-controller \
    && if [ -n "${RUSTC_WRAPPER:-}" ]; then sccache --show-stats; fi \
    && cp target/release/proteus-controller /src/proteus-controller \
    && test -x /src/proteus-controller

# --- Runtime ------------------------------------------------------------
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /src/proteus-controller /usr/local/bin/proteus-controller
USER nonroot:nonroot
EXPOSE 8080
ENV PROTEUS_API_ADDR=0.0.0.0:8080
ENTRYPOINT ["/usr/local/bin/proteus-controller"]
