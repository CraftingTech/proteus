# Deploy Proteus

Kustomize-only install package (no Helm). Local recipes live in the root [`Justfile`](../Justfile)
(`just deploy`, `just image`, `just pf`, `just crds`).

## Layout

```
deploy/
├── Dockerfile
├── kustomization.yaml       # product root → ./base
├── base/                    # namespace, RBAC, Deployment, Service, CRDs
├── crds/                    # generated CRD YAML
├── overlays/
│   └── default/             # pins GHCR :main (tip); release install overrides image
└── examples/
```

## Container image

Published by CI (`.github/workflows/image.yml`) to **`ghcr.io/craftingtech/proteus-controller`**
(`linux/amd64` + `linux/arm64`) on pushes to `main` and on git tags `v*`.

| Trigger | Example image tags |
| ------- | ------------------ |
| Push `main` | `main`, `sha-<short>` |
| Push tag `v0.0.1-alpha.2` | `0.0.1-alpha.2`, `0.0`, `sha-<short>` |

Semver tags omit the leading `v` (docker/metadata-action `pattern={{version}}`). The default
overlay pins **`main`** for tip-of-tree; release installs override the Deployment image to the
semver tag (see below). Image builds also receive `PROTEUS_VERSION` (tag without `v`, else
`0.0.0-dev`) so the binary's `CARGO_PKG_VERSION` matches the release without a git bump.

CI builds each architecture on a **native** GitHub-hosted runner (`ubuntu-latest` /
`ubuntu-24.04-arm`), then merges digests into one multi-arch manifest. That avoids
QEMU emulation (much faster wall-clock) at the cost of ARM Actions minutes. See
`.github/workflows/image.yml`.

### Optional CI cache (BoringCache)

Image and check workflows stay **vendor-free by default** (Buildx + GHA cache /
`Swatinem/rust-cache`). Maintainers may opt into [BoringCache](https://boringcache.com)
for faster warm rebuilds without making forks or contributors depend on it.

Opt-in (repository settings on `CraftingTech/proteus`):

1. Variable `BORINGCACHE_ENABLED` = `true`
2. Secret `BORINGCACHE_RESTORE_TOKEN` (read cache)
3. Secret `BORINGCACHE_SAVE_TOKEN` (write cache on trusted non-PR pushes)

Both the variable **and** at least one token must be present; otherwise CI keeps
the GHA path. Cache failures on the BoringCache path are non-fatal so a successful
GHCR push still finishes green. Workspace config lives in `.boringcache.toml`
(`workspace = "craftingtech/proteus"`). Free-tier limits and account setup are
owned by BoringCache — ask maintainers / see their OSS onboarding docs.

Local build:

```bash
just image
# or:
docker build -f deploy/Dockerfile -t proteus-controller:local .
```

The image builds the Dioxus WASM UI (`dx`) then the Rust controller, and embeds UI assets in the binary.

Backups stream PVC data via a short-lived mount Pod + kube exec `tar` into the operator, which chunks and stores on the fly (no full-archive buffer). On success the Backup status records `durationSeconds` and `throughputBytesPerSec` for later measurement.

## Install with Kustomize

### Tip-of-tree (`main`)

Overlay pins `ghcr.io/craftingtech/proteus-controller:main`:

```bash
just deploy
# or:
kubectl apply -k deploy/overlays/default
```

### Released tag

Tag-first: apply manifests at the git tag, then set the Deployment image to the matching
GHCR semver (no leading `v`). No version-bump commit in the repo.

```bash
TAG=0.0.1-alpha.2
kubectl apply -k "https://github.com/CraftingTech/proteus.git//deploy/overlays/default?ref=v${TAG}"
kubectl -n proteus-system set image deploy/proteus-controller \
  controller=ghcr.io/craftingtech/proteus-controller:${TAG}
```

From a clone checked out at that tag:

```bash
TAG=0.0.1-alpha.2
kubectl apply -k deploy/overlays/default
kubectl -n proteus-system set image deploy/proteus-controller \
  controller=ghcr.io/craftingtech/proteus-controller:${TAG}
```

`deploy/` (product root) also builds without the overlay pin — prefer `overlays/default`.

Local-repo data uses `emptyDir` at `/var/lib/proteus`; replace with a PVC for persistence.

The GHCR package must be **Public** for anonymous / in-cluster pulls without a pull secret.

## Cutting a GitHub release (maintainers)

No bump PR. Product version is the git tag.

1. Make the GHCR package `proteus-controller` **Public** (GitHub → Packages → package settings) if not already.
2. On up-to-date `main`: `git tag -a v0.0.1-alpha.2 -m 'Proteus v0.0.1-alpha.2' && git push origin v0.0.1-alpha.2`
3. CI: `image.yml` publishes multi-arch tags (and bakes `PROTEUS_VERSION` into the binary); `release.yml` opens the GitHub Release with install notes (prerelease when the tag contains `alpha` / `rc` / `beta`).
4. Verify: `docker pull ghcr.io/craftingtech/proteus-controller:0.0.1-alpha.2` (anonymous once Public) and the install snippet from the Release.

Port-forward the UI:

```bash
just pf
open http://127.0.0.1:8080
```

## CRDs

Types live in `crates/proteus-crd`. After changing them:

```bash
just crds
```

This overwrites YAML under `deploy/crds/` from `kube::CustomResourceExt`.

## GitOps consumption (no Helm)

Proteus ships only Kustomize. Consumers can pick either mode:

### A — Remote (Argo points at this repo)

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: proteus
  namespace: platform
spec:
  project: default
  source:
    repoURL: https://github.com/CraftingTech/proteus.git
    targetRevision: v0.0.1-alpha.2   # pin a release git tag
    path: deploy/overlays/default
  destination:
    server: https://kubernetes.default.svc
    namespace: proteus-system
  # Overlay still references :main — pin the release image in a consumer overlay or
  # kustomize `images` patch, e.g. newTag: 0.0.1-alpha.2
  syncPolicy:
    automated:
      selfHeal: true
    syncOptions:
      - CreateNamespace=true
```

### B — Vendor (copy into your GitOps repo)

Copy `deploy/` (or subtree) into your cluster repo, set `images[].newTag` to a GHCR digest/tag
you trust (release semver or digest), then point Argo at **your** path. Same manifests; your repo owns the sync cadence.

Ingress (e.g. Tailscale) is intentionally **not** in this package — add it in a consumer overlay.

## Sample resources

```bash
just samples
```

## Restores

A `ProteusRestore` resolves its source `ProteusBackup` (must be `Succeeded`), opens that backup's
repository, and streams the snapshot's tar bytes into each PVC named in the snapshot, inside
`spec.targetNamespace`.

**Target PVCs must already exist** — M4 does not create or resize PVCs. Pre-provision a PVC per
volume in the target namespace with the same name the backup used, then create the
`ProteusRestore` (via the UI's "New restore" form, or the API). With `spec.overwrite: false`
(default) a non-empty PVC fails the restore instead of silently merging data.

No RBAC changes were needed for restores: the controller already has `pods`/`pods/exec` (create,
exec, delete on a short-lived mount Pod) and `persistentvolumeclaims` `get/list/watch` from the
backup path (`deploy/base/clusterrole.yaml`) — restore only needs to read/exec Pods and
read the target PVC's existence, never create or patch it.

## S3-compatible credentials Secret

From the Proteus UI you can paste Access Key + Secret Key directly; the API creates an Opaque Secret (`<repo>-s3-creds` by default) and sets `credentialsSecretRef`.

Alternatively, create the Secret yourself and reference it. Expected key pairs (first match wins):

| Access key | Secret key |
| ---------- | ---------- |
| `accessKeyId` | `secretAccessKey` |
| `AWS_ACCESS_KEY_ID` | `AWS_SECRET_ACCESS_KEY` |
| `access_key` | `secret_key` |

Example:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: minio-creds
  namespace: proteus-system
type: Opaque
stringData:
  accessKeyId: minio
  secretAccessKey: minio123
---
apiVersion: proteus.io/v1alpha1
kind: ProteusRepository
metadata:
  name: minio-repo
  namespace: proteus-system
spec:
  backend:
    type: s3
    bucket: proteus
    endpoint: http://minio.proteus-system.svc:9000
    region: us-east-1
    forcePathStyle: true
    credentialsSecretRef: minio-creds
```

## Encryption key Secret

From the UI, check "Encrypt at rest" when creating a repository — Proteus generates a random
256-bit key, base64-encodes it, and stores it in a Secret named `<repo>-encryption` (owned by the
repository, so it's GC'd with it). `spec.encryptionSecretRef` is set to that Secret automatically.

To bring your own key instead, create the Secret yourself and pass its name as
`encryptionSecretRef` when creating a repository (via the API — the create call validates the
Secret exists and contains a usable key before creating the CR):

| Secret key | Accepted value |
| ---------- | --------------- |
| `encryptionKey` | 32 raw bytes, or that base64-encoded |
| `ENCRYPTION_KEY` | same, checked if `encryptionKey` is absent |

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: repo-1-encryption
  namespace: proteus-system
type: Opaque
stringData:
  encryptionKey: 6Y2v3q9m0s1z8h5j2k4l7p6q8r0t1u3v5w7x9y1z3a5b7c9d1e3f5g7h9i1j3k5l=
```

Every `ProteusBackup` snapshot chunk is BLAKE3-hashed on plaintext (for content addressing) and,
when the repository has encryption enabled, AES-256-GCM encrypted before it is written to the
object store — the blob on disk/S3 is never plaintext.

### MinIO smoke (optional)

With MinIO listening locally (path-style):

```bash
export PROTEUS_S3_ENDPOINT=http://127.0.0.1:9000
export PROTEUS_S3_ACCESS_KEY=minio
export PROTEUS_S3_SECRET_KEY=minio123
export PROTEUS_S3_BUCKET=proteus
cargo test -p proteus-core --lib live_minio_round_trip_when_configured
```

The test no-ops when `PROTEUS_S3_ENDPOINT` is unset. The reconcile probe uses the same client (`list` under the optional prefix) to set `Ready` / `Failed`.
