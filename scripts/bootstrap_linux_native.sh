#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
CEF_VERSION="146.0.6+g68649e2+chromium-146.0.7680.154"
THIRD_PARTY_DIR="${ROOT_DIR}/third_party/cef"

if [[ "$(uname -s)" != "Linux" ]]; then
  printf 'This bootstrap script only supports Linux.\n' >&2
  exit 1
fi

ARCH="$(uname -m)"
case "${ARCH}" in
  x86_64)
    CEF_SUFFIX="linux64"
    ;;
  aarch64|arm64)
    CEF_SUFFIX="linuxarm64"
    ;;
  *)
    printf 'Unsupported Linux architecture: %s\n' "${ARCH}" >&2
    exit 1
    ;;
esac

CEF_BASENAME="cef_binary_${CEF_VERSION}_${CEF_SUFFIX}"
CEF_ARCHIVE="${CEF_BASENAME}.tar.bz2"
CEF_URL="https://cef-builds.spotifycdn.com/${CEF_ARCHIVE//+/%2B}"
CEF_DEST="${THIRD_PARTY_DIR}/${CEF_BASENAME}"
APT_PACKAGES=(
  build-essential
  clang
  lld
  ninja-build
  pkg-config
  curl
  ca-certificates
  bzip2
  tar
  patchelf
  xvfb
  libasound2-dev
  libatk-bridge2.0-dev
  libatk1.0-dev
  libatspi2.0-dev
  libcairo2-dev
  libcups2-dev
  libdbus-1-dev
  libdrm-dev
  libegl1-mesa-dev
  libgbm-dev
  libgl1-mesa-dev
  libgles2-mesa-dev
  libglib2.0-dev
  gtk3-nocsd
  libgtk-3-dev
  libnss3-dev
  libpango1.0-dev
  libx11-dev
  libx11-xcb-dev
  libxcb-randr0-dev
  libxcb-shm0-dev
  libxcb-xfixes0-dev
  libxcb1-dev
  libxcomposite-dev
  libxdamage-dev
  libxext-dev
  libxfixes-dev
  libxi-dev
  libxkbcommon-dev
  libxkbcommon-x11-dev
  libxrandr-dev
  libxrender-dev
  libxshmfence-dev
  libxtst-dev
)

run_as_root() {
  if [[ "$(id -u)" == "0" ]]; then
    "$@"
  elif command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  else
    printf 'Need root privileges to install apt packages.\n' >&2
    exit 1
  fi
}

printf '==> Installing Linux native dependencies for %s\n' "${ARCH}"
run_as_root apt-get update
run_as_root apt-get install -y "${APT_PACKAGES[@]}"

mkdir -p "${THIRD_PARTY_DIR}"
if [[ ! -d "${CEF_DEST}" ]]; then
  TMP_ARCHIVE="${THIRD_PARTY_DIR}/${CEF_ARCHIVE}"
  printf '==> Downloading %s\n' "${CEF_URL}"
  curl -L --fail --output "${TMP_ARCHIVE}" "${CEF_URL}"
  printf '==> Extracting %s\n' "${CEF_ARCHIVE}"
  tar -C "${THIRD_PARTY_DIR}" -xjf "${TMP_ARCHIVE}"
  rm -f "${TMP_ARCHIVE}"
else
  printf '==> Reusing existing CEF SDK at %s\n' "${CEF_DEST}"
fi

printf '==> Linux native bootstrap complete\n'
printf '    CEF root: %s\n' "${CEF_DEST}"
