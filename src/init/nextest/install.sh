set -euo pipefail
[[ "$GREENSHIFT_RELEASE" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
  { echo "::error::GREENSHIFT_RELEASE is not a release tag (vX.Y.Z)"; exit 1; }
[[ "$GREENSHIFT_SHA256" =~ ^[0-9a-f]{64}$ ]] ||
  { echo "::error::GREENSHIFT_SHA256 is not a SHA-256 checksum"; exit 1; }
bin="$RUNNER_TEMP/greenshift"
curl --fail --location --retry 3 --proto '=https' --proto-redir '=https' --tlsv1.2 \
  "https://github.com/paceheart/greenshift/releases/download/$GREENSHIFT_RELEASE/greenshift-$GREENSHIFT_RELEASE-x86_64-unknown-linux-musl" \
  --output "$bin.download"
# Fail closed: only a binary matching the pinned checksum is installed.
if ! printf '%s  %s\n' "$GREENSHIFT_SHA256" "$bin.download" | sha256sum --check --strict; then
  rm -f "$bin.download"
  echo "::error::greenshift $GREENSHIFT_RELEASE does not match the pinned SHA-256; not installed"
  exit 1
fi
chmod +x "$bin.download"
mv "$bin.download" "$bin"
