#!/usr/bin/env bash
# Comprueba que la etiqueta de git es "v" + la versión de Cargo.toml.
# Uso: check-version.sh <etiqueta> [ruta a Cargo.toml]
# Sale con 0 e imprime prerelease=true|false; con 1 y un mensaje en stderr si no coinciden.
tag="${1:?uso: check-version.sh <etiqueta> [Cargo.toml]}"
manifest="${2:-crates/temiktli-shell/Cargo.toml}"

version="$(sed -n 's/^version *= *"\(.*\)"$/\1/p' "$manifest" | head -n 1)"
if [ -z "$version" ]; then
  echo "no se encontró la versión en $manifest" >&2
  exit 1
fi
if [ "$tag" != "v$version" ]; then
  echo "la etiqueta '$tag' no coincide con la versión de Cargo.toml (esperada 'v$version')" >&2
  exit 1
fi
case "$version" in
  *-*) echo "prerelease=true" ;;
  *)   echo "prerelease=false" ;;
esac
