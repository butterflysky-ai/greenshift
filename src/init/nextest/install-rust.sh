set -euo pipefail
if command -v cargo >/dev/null; then
  cargo --version
  exit 0
fi
# Forgejo's default runner image has no Rust: install it with rustup,
# pinned by version and checksum. rustup then follows the repository's
# rust-toolchain.toml, if it has one.
init="$RUNNER_TEMP/rustup-init"
curl --fail --location --retry 3 --proto '=https' --proto-redir '=https' --tlsv1.2 \
  "https://static.rust-lang.org/rustup/archive/@RUSTUP@/x86_64-unknown-linux-gnu/rustup-init" \
  --output "$init"
if ! printf '%s  %s\n' "@RUSTUP_SHA256@" "$init" | sha256sum --check --strict; then
  rm -f "$init"
  echo "::error::rustup-init @RUSTUP@ does not match its pinned SHA-256; not installed"
  exit 1
fi
chmod +x "$init"
"$init" -y --no-modify-path --profile minimal
rm -f "$init"
echo "${CARGO_HOME:-$HOME/.cargo}/bin" >> "$GITHUB_PATH"
