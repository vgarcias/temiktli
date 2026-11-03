#!/usr/bin/env bash
# Copia lo que genera cargo-packager (dist/packager) a dist/ con los nombres de la spec.
# Uso: scripts/collect-artifacts.sh <windows|macos|linux> <arch: x64|arm64> <versión SemVer>
set -euo pipefail

os="${1:?uso: collect-artifacts.sh <windows|macos|linux> <arch> <versión>}"
arch="${2:?falta la arquitectura}"
ver="${3:?falta la versión}"
src="dist/packager"
out="dist"
mkdir -p "$out"

take() { # take <patrón> <nombre final>
  local f
  f=$(ls "$src"/$1 2>/dev/null | head -n 1 || true)
  [ -n "$f" ] || { echo "no se encontró $src/$1" >&2; return 1; }
  cp "$f" "$out/$2"
  echo "$out/$2"
}

case "$os" in
  windows) take "*-setup.exe" "Temiktli-$ver-windows-$arch-setup.exe" ;;
  macos)   take "*.dmg" "Temiktli-$ver-macos-$arch.dmg" ;;
  linux)
    take "*.deb" "Temiktli-$ver-linux-$arch.deb"
    take "*.AppImage" "Temiktli-$ver-linux-$arch.AppImage" || echo "AppImage no generado (opcional)" >&2
    ;;
  *) echo "sistema desconocido: $os" >&2; exit 2 ;;
esac
