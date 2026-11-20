set -euo pipefail
if command -v cargo >/dev/null; then
  cargo --version
  exit 0
fi
# Forgejo's default runner image has no Rust: install it with rustup,
# pinned by version and checksum. rustup then follows the repository's
# rust-toolchain.toml, if it has one. The SHA-256 is of the
# x86_64-unknown-linux-gnu rustup-init, as static.rust-lang.org publishes it.
rustup_version=1.29.3
rustup_sha256=b1c8cb14e30889e7e2999e6b69e62d9e9b9ffc0f4df8180564d306d9680e6c55
init="$RUNNER_TEMP/rustup-init"
curl --fail --location --retry 3 --proto '=https' --proto-redir '=https' --tlsv1.2 \
  --remove-on-error \
  "https://static.rust-lang.org/rustup/archive/$rustup_version/x86_64-unknown-linux-gnu/rustup-init" \
  --output "$init"
if ! printf '%s  %s\n' "$rustup_sha256" "$init" | sha256sum --check --strict; then
  rm -f "$init"
  echo "::error::rustup-init $rustup_version does not match its pinned SHA-256; not installed"
  exit 1
fi
chmod +x "$init"
"$init" -y --no-modify-path --profile minimal
rm -f "$init"
echo "${CARGO_HOME:-$HOME/.cargo}/bin" >> "$GITHUB_PATH"
