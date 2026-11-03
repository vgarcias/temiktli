#!/usr/bin/env bash
# Pruebas de check-version.sh. Uso: bash scripts/check-version.test.sh
here="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d)"
fail=0

run() { # run <versión de Cargo> <etiqueta> <código esperado> <salida esperada o ''>
  printf '[package]\nname = "x"\nversion = "%s"\n' "$1" > "$tmp/Cargo.toml"
  out="$(bash "$here/check-version.sh" "$2" "$tmp/Cargo.toml" 2>/dev/null)"; code=$?
  if [ "$code" != "$3" ] || { [ -n "$4" ] && [ "$out" != "$4" ]; }; then
    echo "FALLO: cargo=$1 tag=$2 -> código $code, salida '$out' (esperado $3 '$4')"; fail=1
  else
    echo "ok: cargo=$1 tag=$2"
  fi
}

run 0.1.0-rc.1 v0.1.0-rc.1 0 prerelease=true
run 0.1.0-rc.1 v0.1.0 1 ''
run 0.1.0-rc.1 0.1.0-rc.1 1 ''
run 0.1.0 v0.1.0 0 prerelease=false
run 0.1.0 v0.1.0-rc.1 1 ''
rm -rf "$tmp"
exit $fail
