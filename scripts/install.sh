#!/bin/sh
# Installs the latest owtui release (or the version given) for this platform:
#   curl -fsSL https://raw.githubusercontent.com/tlpinney/owtui/main/scripts/install.sh | sh
#   sh install.sh v0.1.0
# It installs only after verifying that
#   1. SHA256SUMS carries a valid SSH signature from the maintainer key below
#      (made offline by the maintainer, not by CI), and
#   2. the downloaded archive matches its line in SHA256SUMS.
# A failed check stops the install. Set OWTUI_INSTALL_DIR to choose where the
# binary goes (default ~/.local/bin).
set -eu

repo="tlpinney/owtui"
namespace="owtui-release"
signer="tlpinney"
# Same key as signing/allowed_signers in the repository.
allowed='tlpinney namespaces="owtui-release" ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFSZDt6uNCxuvSttHDBUcV1dctXJ1DBwYnQdbk19FfFP'
dest_dir="${OWTUI_INSTALL_DIR:-$HOME/.local/bin}"
version="${1:-latest}"

fail() {
  echo "owtui: $1" >&2
  exit 1
}

case "$(uname -s)" in
  Darwin) os=macos ;;
  Linux) os=linux ;;
  *) fail "no prebuilt binary for $(uname -s); build from source with cargo install --git https://github.com/$repo" ;;
esac
case "$(uname -m)" in
  arm64 | aarch64) arch=aarch64 ;;
  x86_64 | amd64) arch=x86_64 ;;
  *) fail "no prebuilt binary for $(uname -m)" ;;
esac
command -v ssh-keygen >/dev/null 2>&1 || fail "ssh-keygen (OpenSSH) is needed to verify the release signature"

asset="owtui-$os-$arch.tar.gz"
if [ "$version" = latest ]; then
  base="https://github.com/$repo/releases/latest/download"
else
  base="https://github.com/$repo/releases/download/$version"
fi

download() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --proto '=https' --tlsv1.2 --retry 2 -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q --https-only -O "$2" "$1"
  else
    fail "curl or wget is needed to download the release"
  fi
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    fail "sha256sum or shasum is needed to verify the download"
  fi
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

download "$base/SHA256SUMS" "$tmp/SHA256SUMS" || fail "no release found ($version)"
download "$base/SHA256SUMS.sig" "$tmp/SHA256SUMS.sig" || fail "the release has no signature; refusing to install"
download "$base/$asset" "$tmp/$asset" || fail "the release has no $asset"

printf '%s\n' "$allowed" > "$tmp/allowed_signers"
ssh-keygen -Y verify -f "$tmp/allowed_signers" -I "$signer" -n "$namespace" \
  -s "$tmp/SHA256SUMS.sig" < "$tmp/SHA256SUMS" >/dev/null 2>&1 \
  || fail "SHA256SUMS signature does not verify; refusing to install"

expected="$(awk -v f="$asset" '$2 == f || $2 == "*" f { print $1 }' "$tmp/SHA256SUMS")"
[ -n "$expected" ] || fail "$asset is not listed in the signed SHA256SUMS"
actual="$(sha256 "$tmp/$asset")"
[ "$expected" = "$actual" ] || fail "checksum mismatch for $asset (expected $expected, got $actual)"

# Extract only the binary, never other archive members.
tar -xzf "$tmp/$asset" -C "$tmp" owtui
mkdir -p "$dest_dir"
mv "$tmp/owtui" "$dest_dir/owtui"
chmod 755 "$dest_dir/owtui"
echo "owtui: installed signed $("$dest_dir/owtui" --version | cut -d' ' -f2) for $os-$arch to $dest_dir/owtui"
case ":$PATH:" in
  *":$dest_dir:"*) ;;
  *) echo "owtui: add $dest_dir to your PATH to run it as owtui" ;;
esac
