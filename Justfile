# Proteus — local workflows
# https://github.com/casey/just
#
#   just              # list recipes
#   just check        # pre-commit gates
#   just run          # controller + embedded UI (~/.kube/config)
#   just run kubeconfig=/path/to/config

set shell := ["bash", "-euo", "pipefail", "-c"]
set dotenv-load := false

export PROTEUS_API_ADDR := env_var_or_default("PROTEUS_API_ADDR", "0.0.0.0:8080")

# Default kubeconfig: $KUBECONFIG if set, else ~/.kube/config
home := env_var("HOME")
default_kubeconfig := home / ".kube/config"
kubeconfig := env_var_or_default("KUBECONFIG", default_kubeconfig)

image := env_var_or_default("PROTEUS_IMAGE", "proteus-controller:local")
kustomize := "deploy/overlays/default"
namespace := "proteus-system"

default:
    @just --list

# --- quality gates ----------------------------------------------------------

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --check

clippy:
    cargo clippy --workspace --all-targets --exclude proteus-ui -- -D warnings

test:
    cargo test --workspace --exclude proteus-ui

# Pre-commit: format check + clippy + tests + UI bundle
check: fmt-check clippy test build-ui

# --- UI (Dioxus) ------------------------------------------------------------

# Build WASM UI and stage assets into crates/proteus-ui/dist for rust-embed
build-ui:
    #!/usr/bin/env bash
    set -euo pipefail
    # Dioxus installs its own workspace wrapper, so an outer compiler wrapper
    # must be cleared for this command only.
    RUSTC_WRAPPER= dx build -p proteus-ui --platform web --release
    mkdir -p crates/proteus-ui/dist
    rm -rf crates/proteus-ui/dist/*
    src="${CARGO_TARGET_DIR:-target}/dx/proteus-ui/release/web/public"
    if [[ ! -f "$src/index.html" ]]; then
      echo "dx public output missing at $src" >&2
      exit 1
    fi
    cp -a "$src"/. crates/proteus-ui/dist/
    echo "UI assets → crates/proteus-ui/dist"

# Hot-reload UI on :5173 — needs the controller API on :8080 in another terminal (`just run`).
# Embedded path (`just run` alone) uses relative /api URLs; no CORS needed.
ui:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "UI hot-reload → http://127.0.0.1:5173"
    echo "API expected  → http://127.0.0.1:8080  (run: just run)"
    PROTEUS_API_BASE=http://127.0.0.1:8080 dx serve -p proteus-ui --platform web --port 5173 --open false

# --- CRDs -------------------------------------------------------------------

# Regenerate deploy/crds from Rust types
crds:
    cargo run -q -p proteus-crd --bin proteus-crdgen -- deploy/crds

# Apply CRDs to the current kube context and wait until Established
ensure-crds kubeconfig=kubeconfig: crds
    #!/usr/bin/env bash
    set -euo pipefail
    export KUBECONFIG="{{kubeconfig}}"
    if [[ ! -f "$KUBECONFIG" ]]; then
      echo "kubeconfig not found: $KUBECONFIG" >&2
      exit 1
    fi
    echo "Applying CRDs (kubeconfig: $KUBECONFIG)"
    kubectl apply -k deploy/crds
    for crd in \
      proteusrepositories.proteus.io \
      proteusbackups.proteus.io \
      proteusrestores.proteus.io
    do
      kubectl wait --for=condition=Established "crd/$crd" --timeout=60s
    done
    echo "CRDs ready"

# Remove Proteus CRs + CRDs from the current kube context
cleanup kubeconfig=kubeconfig:
    #!/usr/bin/env bash
    set -euo pipefail
    export KUBECONFIG="{{kubeconfig}}"
    if [[ ! -f "$KUBECONFIG" ]]; then
      echo "kubeconfig not found: $KUBECONFIG" >&2
      exit 1
    fi
    echo "Cleaning Proteus from cluster (kubeconfig: $KUBECONFIG)"
    kubectl delete -f deploy/examples/sample-resources.yaml --ignore-not-found
    kubectl delete -k deploy/crds --ignore-not-found
    echo "Cleanup done"

# --- run locally ------------------------------------------------------------

# Build UI, ensure CRDs on cluster, then run controller (API + UI on PROTEUS_API_ADDR).
# Default kubeconfig: ~/.kube/config (or $KUBECONFIG).
# Override: just run kubeconfig=/path/to/config
run kubeconfig=kubeconfig: build-ui (ensure-crds kubeconfig)
    #!/usr/bin/env bash
    set -euo pipefail
    export KUBECONFIG="{{kubeconfig}}"
    if [[ ! -f "$KUBECONFIG" ]]; then
      echo "kubeconfig not found: $KUBECONFIG" >&2
      echo "pass one with: just run kubeconfig=/path/to/config" >&2
      exit 1
    fi
    echo "Using kubeconfig: $KUBECONFIG"
    cargo run -p proteus-controller

# Hit local health/cluster endpoints (controller must be up)
smoke:
    curl -sf "http://127.0.0.1:8080/healthz"
    echo
    curl -sf "http://127.0.0.1:8080/api/v1/cluster"
    echo

# --- cluster (kustomize) ----------------------------------------------------

# Apply CRDs + controller into the current kube context
# Override: just deploy kubeconfig=/path/to/config
deploy kubeconfig=kubeconfig:
    #!/usr/bin/env bash
    set -euo pipefail
    export KUBECONFIG="{{kubeconfig}}"
    kubectl apply -k {{kustomize}}

undeploy kubeconfig=kubeconfig:
    #!/usr/bin/env bash
    set -euo pipefail
    export KUBECONFIG="{{kubeconfig}}"
    kubectl delete -k {{kustomize}}

# Port-forward Service → localhost:8080
pf kubeconfig=kubeconfig:
    #!/usr/bin/env bash
    set -euo pipefail
    export KUBECONFIG="{{kubeconfig}}"
    kubectl -n {{namespace}} port-forward svc/proteus 8080:80

# Sample CRs (repos/backups) — optional
samples kubeconfig=kubeconfig:
    #!/usr/bin/env bash
    set -euo pipefail
    export KUBECONFIG="{{kubeconfig}}"
    kubectl apply -f deploy/examples/sample-resources.yaml

# --- release / image --------------------------------------------------------

build-release:
    cargo build --release -p proteus-controller

# Multi-stage image (Dioxus + controller)
image:
    docker build -f deploy/Dockerfile -t {{image}} .

# Build image then deploy (set PROTEUS_IMAGE if you push to a registry)
ship kubeconfig=kubeconfig: image
    #!/usr/bin/env bash
    set -euo pipefail
    export KUBECONFIG="{{kubeconfig}}"
    kubectl apply -k {{kustomize}}
