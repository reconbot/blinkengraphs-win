#!/usr/bin/env bash
# Usage: installer/build.sh <x64|arm64> <path to blinkengraphs.exe> <out dir>
set -euo pipefail
arch=$1 exe=$2 out=$3
here=$(cd "$(dirname "$0")" && pwd)
version=$(cargo pkgid -p blinkengraphs | sed 's/.*[#@]//')
command -v wix >/dev/null || dotnet tool install --global wix --version 7.0.1
export PATH="$PATH:$HOME/.dotnet/tools"
wix extension add -g WixToolset.UI.wixext/7.0.1 WixToolset.Util.wixext/7.0.1
mkdir -p "$out"
# WiX v7 requires accepting its EULA; see https://docs.firegiant.com/wix/osmf/
wix build "$here/blinkengraphs.wxs" -arch "$arch" -acceptEula wix7 \
  -ext WixToolset.UI.wixext -ext WixToolset.Util.wixext \
  -d Version="$version" -d Exe="$exe" -d License="$here/license.rtf" \
  -o "$out/blinkengraphs-$arch.msi"
