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

`ProteusRepository` backends of `type: s3` reference a Secret via `credentialsSecretRef` in the same namespace. Expected key pairs (first match wins):

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
```
