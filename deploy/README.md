# Deploy Proteus

All of this is also available via the root [`Justfile`](../Justfile) (`just deploy`, `just image`, `just pf`, `just crds`).

## Container image

```bash
just image
# or:
docker build -f deploy/Dockerfile -t proteus-controller:local .
```

The image builds the Dioxus WASM UI (`dx`) then the Rust controller, and embeds UI assets in the binary.

## Install with Kustomize

```bash
just deploy
# or:
kubectl apply -k deploy/kustomize/overlays/default
```

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

This overwrites YAML under `deploy/kustomize/crds/` from `kube::CustomResourceExt`.

## Sample resources

```bash
just samples
```

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
