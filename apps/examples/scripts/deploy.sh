#!/usr/bin/env bash
# Deploy the static examples app to a GCS bucket. The whole app is static — no
# server — so this is a plain object upload: the built shell, the Q4_K_M GGUF,
# and the seeded embedding cache.
#
# Usage:
#   VRULES_EXAMPLES_BUCKET=gs://my-bucket ./scripts/deploy.sh
#   ./scripts/deploy.sh gs://my-bucket
#
# One-time bucket setup (public static website), if not already done:
#   gcloud storage buckets create gs://my-bucket --location=US
#   gcloud storage buckets update gs://my-bucket --web-main-page-suffix=index.html
#   gcloud storage buckets add-iam-policy-binding gs://my-bucket \
#     --member=allUsers --role=roles/storage.objectViewer
set -euo pipefail

BUCKET="${1:-${VRULES_EXAMPLES_BUCKET:-}}"
if [[ -z "$BUCKET" ]]; then
  echo "error: pass a bucket (gs://...) as \$1 or set VRULES_EXAMPLES_BUCKET" >&2
  exit 1
fi

cd "$(dirname "$0")/.."

# Seed the embedding cache if it is missing (regenerable, gitignored).
if [[ -z "$(find public/vrules-rest -type f 2>/dev/null | head -1)" ]]; then
  echo "seeding embedding cache…"
  npm run seed
fi

echo "building…"
npm run build   # emits dist/ including public/ (model gguf + seeded cache)

echo "uploading to $BUCKET…"
gcloud storage rsync --recursive --delete-unmatched-destination-objects dist "$BUCKET"

# Hashed assets, the model, and content-addressed cache blobs are immutable;
# the entry points must revalidate so updates propagate.
echo "setting cache headers…"
gcloud storage objects update "$BUCKET/assets/**" \
  --cache-control="public, max-age=31536000, immutable" >/dev/null 2>&1 || true
gcloud storage objects update "$BUCKET/models/**" "$BUCKET/vrules-rest/**" \
  --content-type="application/octet-stream" \
  --cache-control="public, max-age=31536000, immutable" >/dev/null 2>&1 || true
gcloud storage objects update "$BUCKET/index.html" "$BUCKET/models/model.json" \
  --cache-control="no-cache" >/dev/null 2>&1 || true

echo "done. If the bucket is a public website, it serves at:"
echo "  https://storage.googleapis.com/${BUCKET#gs://}/index.html"
