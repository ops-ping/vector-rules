#!/usr/bin/env bash
# Fetch the EmbeddingGemma GGUF model into the vrules model dir, verifying its
# SHA256. The model is a standard, separately-versioned artifact — never built by
# us and reused across every vrules release. Idempotent: a verified file is kept.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=pins.env
source "$here/pins.env"

dest_dir="${VRULES_MODEL_DIR:-$HOME/.local/share/vrules/models}"
dest="$dest_dir/$MODEL_FILE"
url="${MODEL_URL:?MODEL_URL unset}"

if [[ -z "${MODEL_SHA256:-}" ]]; then
  echo "fetch-model: MODEL_SHA256 is not pinned in release/pins.env — refusing to" \
       "fetch an unverified model. Fill it at pin time." >&2
  exit 1
fi

verify() { echo "$MODEL_SHA256  $1" | sha256sum -c - >/dev/null 2>&1; }

if [[ -f "$dest" ]] && verify "$dest"; then
  echo "fetch-model: $dest already present and verified."
  exit 0
fi

mkdir -p "$dest_dir"
echo "fetch-model: downloading $MODEL_FILE -> $dest"
auth=()
[[ -n "${HF_TOKEN:-}" ]] && auth=(--header "Authorization: Bearer $HF_TOKEN")
curl -fL --retry 3 "${auth[@]}" -o "$dest.part" "$url"
mv "$dest.part" "$dest"

if ! verify "$dest"; then
  echo "fetch-model: SHA256 mismatch for $dest (expected $MODEL_SHA256)." >&2
  echo "  got: $(sha256sum "$dest" | cut -d' ' -f1)" >&2
  exit 1
fi
echo "fetch-model: verified $dest"
