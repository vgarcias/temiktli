# Distribución de Temiktli — plan de implementación

> **Para quien ejecute:** SUB-SKILL REQUERIDA: usar superpowers:subagent-driven-development (recomendado) o superpowers:executing-plans para implementar este plan tarea por tarea. Los pasos usan casillas (`- [ ]`).

**Objetivo:** que cualquiera descargue de GitHub un instalador de Temiktli para Windows, macOS o Linux, construido automáticamente al crear una etiqueta de versión.

**Arquitectura:** primero se deja el código portable y verificado por CI en las tres plataformas; después se añaden iconos, la configuración de `cargo-packager`, el flujo de *release* y la documentación. Un único ejecutable con todos los recursos embebidos, así que el instalador no lleva archivos aparte.

**Stack:** Rust (edición 2024), `wry` 0.57 + `tao` 0.37, `cargo-packager`, GitHub Actions, crate `dirs`, Node (tests JS), Bash.

**Spec:** `docs/superpowers/specs/2026-11-02-distribucion-design.md` (aprobada por el usuario). Leerla antes de empezar.

## Restricciones globales (de la spec, literales)

- Plataformas: Windows x64, macOS Apple Silicon e Intel, Linux x64. Sin ARM de Windows/Linux.
- Licencia **MIT** (`LICENSE`, `license = "MIT"` en ambos `Cargo.toml`). Usuario de GitHub `vgarcias`, repositorio `temiktli`, identificador `io.github.vgarcias.temiktli` (no puede cambiar tras el primer *release* público).
- Sin firma; en macOS, firma **ad-hoc** (`codesign --sign -`) sobre el `.app` **antes** de crear el `.dmg`, en las dos arquitecturas.
- Sin actualizador. Los datos del usuario (`snippets.json`, `config.json`) nunca se tocan al instalar, actualizar o desinstalar. Carpeta de datos: `%APPDATA%\Temiktli` (Windows, sin cambios), `~/Library/Application Support/Temiktli` (macOS), `$XDG_CONFIG_HOME` o `~/.config/Temiktli` (Linux).
- Artefactos: `Temiktli-<ver>-windows-x64-setup.exe`, `Temiktli-<ver>-macos-arm64.dmg`, `Temiktli-<ver>-macos-x64.dmg`, `Temiktli-<ver>-linux-x64.deb`, `Temiktli-<ver>-linux-x64.AppImage` (opcional), más `SHA256SUMS.txt`.
- Versión de `Cargo.toml` en SemVer completo (`0.1.0-rc.1`); etiqueta = `v` + esa versión; el flujo falla si no coinciden; versión con guion ⇒ *pre-release*; en el `.deb` el guion pasa a `~`.
- Runners: `windows-latest`, `macos-latest` (Intel por `--target x86_64-apple-darwin`), `ubuntu-22.04`. Linux instala `libwebkit2gtk-4.1-dev` y `libgtk-3-dev`.
- Todo texto nuevo de interfaz va por clave en los siete `locales/*.json` (los tests fallan si falta una o queda español literal).
- Reglas del proyecto: no automatizar ratón/teclado ni lanzar la GUI; `cargo` no está en el PATH de Bash (`export PATH="/c/Users/v_gar/.cargo/bin:$PATH"`); Bash falla con heredocs de ~30 KB (usar Write/Edit); tests JS uno por uno con `node crates/temiktli-shell/assets/<nombre>.test.js`; `cargo build` sin warnings.
- Hook GateGuard: la primera edición de cada archivo exige exponer antes quién lo llama, qué afecta y la instrucción literal del usuario.

## Review Focus (casos que la spec implica y ninguna tarea cubre por sí sola)

1. Perfil de usuario con espacios o letras no ASCII (`C:\Users\v_gar`, un Mac con «José»): la carpeta de datos se resuelve y se escribe bien. *(Tarea 2)*
2. Sin ninguna variable de entorno de carpeta de datos: `storage_path()` devuelve `None` y la aplicación arranca igual con configuración vacía. *(Tarea 2)*
3. Usuario de Windows que ya tiene datos: tras el cambio sigue leyendo el mismo `snippets.json`. *(Tarea 2)*
4. Etiqueta que no coincide con la versión de `Cargo.toml`, o versión preliminar mal escrita: el flujo falla antes de construir. *(Tarea 6)*
5. Instalar una versión encima de otra, o desinstalar: los datos siguen ahí; y el `.deb` declara WebKitGTK como dependencia para quien no la tiene. *(Tareas 5 y 8)*

---

## Mapa de archivos

- Crear: `.gitignore`, `LICENSE`, `README.md`, `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `scripts/check-version.sh`, `scripts/check-version.test.sh`, `scripts/collect-artifacts.sh`, `scripts/make-icons.py`, `crates/temiktli-shell/assets/icon.icns`, `crates/temiktli-shell/assets/icon-512.png`, `docs/release-notes-template.md`.
- Modificar: `Cargo.toml` de ambos crates (metadatos), `crates/temiktli-shell/src/snippets.rs` (carpeta de datos), `crates/temiktli-shell/src/main.rs` (mensaje de error de idioma ~l.1818), `crates/temiktli-shell/locales/*.json` (textos que nombran Windows).

---

### Tarea 1: Repositorio, licencia y metadatos

**Archivos:** crear `.gitignore`, `LICENSE`; modificar los `Cargo.toml` de `temiktli-core` y `temiktli-shell`.

**Interfaces:** produce un repositorio git local con un primer commit limpio, sin archivos ajenos.

- [ ] **Paso 1:** `git init -b main` en `D:\otros\rea`.
- [ ] **Paso 2:** crear `.gitignore` con `target/`, `dist/`, `/Captura*.png`, `/*.mp4`, `/content-large_groups*` y `/handoff-*.md`. `logo.jpg` se versiona (lo usa el script de iconos).
- [ ] **Paso 3:** crear `LICENSE` con el texto MIT estándar, año 2026, titular «vgarcias».
- [ ] **Paso 4:** en ambos crates añadir `license = "MIT"`, `repository = "https://github.com/vgarcias/temiktli"`, `description` y `authors = ["vgarcias"]`; `version = "0.1.0-rc.1"` en los dos, y la misma versión en la dependencia `temiktli-core` del shell.
- [ ] **Paso 5:** verificar: `cargo build 2>&1 | grep -E "^(error|warning)"` no imprime nada, y `git status --short` no lista capturas, vídeo ni `handoff-*`.
- [ ] **Paso 6:** commit `chore: repositorio, licencia MIT y metadatos`.

---

### Tarea 2: Carpeta de datos por sistema

**Archivos:** modificar `crates/temiktli-shell/Cargo.toml` (`dirs = "6"`), `crates/temiktli-shell/src/snippets.rs:35-41` y `crates/temiktli-shell/src/main.rs` (~l.1818); pruebas en el módulo `tests` de `snippets.rs`.

**Interfaces:**
- Mantiene `pub fn storage_path() -> Option<PathBuf>` (la usa `config::storage_path`) devolviendo `<config_dir>/Temiktli/snippets.json`.
- Añade privada `fn storage_path_in(config_dir: Option<PathBuf>) -> Option<PathBuf>` (pura, para probar).

- [ ] **Paso 1: tests que fallan** en `snippets.rs`: `storage_path_in_appends_app_folder` (`Some("/x/y")` ⇒ `/x/y/Temiktli/snippets.json`), `storage_path_in_none_gives_none`, `storage_path_handles_spaces_and_non_ascii` (base `"/tmp/José Pérez"` se conserva íntegra) y, con `#[cfg(windows)]`, `windows_data_dir_is_still_appdata` (`storage_path()` == `%APPDATA%\Temiktli\snippets.json`, la ruta que ya usa quien tiene datos).
- [ ] **Paso 2:** `cargo test -p temiktli-shell storage_path 2>&1 | grep -E "test result|FAILED|error"`. Esperado: error de compilación (función inexistente).
- [ ] **Paso 3:** implementar `storage_path_in` y hacer que `storage_path()` llame `storage_path_in(dirs::config_dir())`; eliminar la búsqueda manual de `APPDATA`/`XDG_CONFIG_HOME`/`HOME`. En macOS `dirs::config_dir()` ya es `~/Library/Application Support`.
- [ ] **Paso 4:** en `main.rs` el aviso `status.language_save_failed` ya no fija la cadena `"APPDATA"`: pasar el motivo real del error de `config::save` (leer el contexto de la línea). Si la clave cambia de parámetros, actualizar los siete JSON.
- [ ] **Paso 5:** `cargo test --workspace 2>&1 | grep -E "^(error|warning)|test result|FAILED|panicked"` todo en verde; `cargo build` sin warnings.
- [ ] **Paso 6:** commit `fix: carpeta de datos estándar en macOS y Linux`.

---

### Tarea 3: CI en las tres plataformas y portabilidad

**Archivos:** crear `.github/workflows/ci.yml`; modificar lo que falle en macOS/Linux; revisar `locales/*.json` por textos que dicen «Windows».

**Interfaces:** produce un flujo `CI` con matriz `[windows-latest, macos-latest, ubuntu-22.04]`. Es la **fase previa** de la spec: nada de empaquetado hasta que esté en verde.

- [ ] **Paso 1:** crear `ci.yml` (en `push` y `pull_request`): instala Rust estable; en Linux `sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev`; ejecuta `cargo build`, `cargo test --workspace` y `for f in crates/temiktli-shell/assets/*.test.js; do node "$f" || exit 1; done`. `RUSTFLAGS: -D warnings`.
- [ ] **Paso 2:** buscar textos visibles que nombren Windows (`grep -rniE "windows" crates/temiktli-shell/locales crates/temiktli-shell/assets/*.html crates/temiktli-shell/src/main.rs`) y hacerlos neutros en los siete JSON; los comentarios de código pueden quedarse.
- [ ] **Paso 3 (acción hacia fuera, pedir confirmación explícita al usuario):** crear el repositorio público y subir `main`: `gh repo create vgarcias/temiktli --public --source . --push`.
- [ ] **Paso 4:** `gh run watch`. Por cada fallo en macOS o Linux, arreglar la **causa** (sin `#[allow]` ni `cfg` para silenciar, salvo avisos de verdad inaplicables, con comentario) y empujar de nuevo, hasta tener las tres en verde.
- [ ] **Paso 5:** commits `ci: pruebas en Windows, macOS y Linux` y uno por cada arreglo.

---

### Tarea 4: Iconos de macOS y Linux

**Archivos:** crear `scripts/make-icons.py`, `crates/temiktli-shell/assets/icon.icns`, `crates/temiktli-shell/assets/icon-512.png`; prueba en el módulo `tests` de `crates/temiktli-shell/src/main.rs`.

**Interfaces:** produce `icon.icns` (cabecera `icns`) e `icon-512.png` (≥512 px). Los consume la configuración del empaquetador (Tarea 5).

- [ ] **Paso 1: test que falla** `packaging_icons_exist_and_are_valid`: `include_bytes!("../assets/icon.icns")` empieza por `b"icns"`; `icon-512.png` empieza por la firma PNG y su ancho (bytes 16..20, IHDR) es ≥ 512.
- [ ] **Paso 2:** `cargo test -p temiktli-shell packaging_icons 2>&1 | grep -E "test result|error"`; esperado: error de compilación (archivos ausentes).
- [ ] **Paso 3:** escribir `make-icons.py` (Pillow) que parte de `assets/icon.png`, o de `logo.jpg` si no llega a 512 px, y escribe ambos archivos; ejecutarlo y versionar la salida.
- [ ] **Paso 4:** el test pasa y `cargo build` sigue sin avisos.
- [ ] **Paso 5:** commit `feat: iconos para macOS y Linux`.

---

### Tarea 5: Configuración de `cargo-packager` y script de artefactos

**Archivos:** modificar `crates/temiktli-shell/Cargo.toml` (`[package.metadata.packager]`); crear `scripts/collect-artifacts.sh`.

**Interfaces:**
- Configuración: `product-name = "Temiktli"`, `identifier = "io.github.vgarcias.temiktli"`, formatos por sistema (`nsis`, `dmg`, `deb` y `appimage`), iconos de la Tarea 4 e `icon.ico`, NSIS por usuario (`install-mode = "currentUser"`) con WebView2 por *bootstrapper* pequeño, y `.deb` con dependencias `libwebkit2gtk-4.1-0` y `libgtk-3-0`.
- `scripts/collect-artifacts.sh <windows|macos|linux> <arch> <version>`: copia la salida del empaquetador a `dist/` con los nombres de la spec; el `.deb` lleva la versión con `~` en lugar de `-`.

- [ ] **Paso 1:** `cargo install cargo-packager --locked`; escribir la configuración (claves exactas según `cargo packager --help` y la documentación de la versión instalada).
- [ ] **Paso 2:** en esta máquina Windows, `cargo packager --release -p temiktli-shell --formats nsis`. Esperado: un `.exe` instalador.
- [ ] **Paso 3:** escribir el script y ejecutarlo para `windows x64 0.1.0-rc.1`. Esperado: `dist/Temiktli-0.1.0-rc.1-windows-x64-setup.exe`.
- [ ] **Paso 4:** la instalación visual y el icono los comprueba el usuario (no se lanza la GUI); aquí solo se verifica que el archivo existe y tiene tamaño razonable.
- [ ] **Paso 5:** commit `build: configuración de cargo-packager`.

---

### Tarea 6: Flujo de *release*

**Archivos:** crear `.github/workflows/release.yml`, `scripts/check-version.sh`, `scripts/check-version.test.sh`.

**Interfaces:**
- `scripts/check-version.sh <tag>`: sale con 0 si `<tag>` == `v` + versión de `crates/temiktli-shell/Cargo.toml`; con 1 y mensaje si no; imprime `prerelease=true|false` (true si la versión tiene guion).

- [ ] **Paso 1: pruebas que fallan** en `check-version.test.sh`: `v0.1.0-rc.1` con Cargo `0.1.0-rc.1` ⇒ 0 y `prerelease=true`; `v0.1.0` con Cargo `0.1.0-rc.1` ⇒ 1; `0.1.0-rc.1` sin `v` ⇒ 1; `v0.1.0` con Cargo `0.1.0` ⇒ 0 y `prerelease=false`. Ejecutarlo: esperado FALLO (script ausente).
- [ ] **Paso 2:** implementar `check-version.sh`; las pruebas pasan.
- [ ] **Paso 3:** `release.yml`, disparado por `push` de etiquetas `v*`: trabajo `check` (el script) → matriz de construcción que repite los tests de `ci.yml` y luego `cargo packager`; en macOS compilar ambos objetivos y **firmar el `.app` con `codesign --force --deep --sign -` antes de crear el `.dmg`** (si el empaquetador no permite el paso intermedio, construir el `.app`, firmar y crear el `.dmg` con `hdiutil`) → `collect-artifacts.sh` → subir artefactos → trabajo final que genera `SHA256SUMS.txt` y crea un **borrador** de *Release* (`gh release create --draft`, con `--prerelease` según el script y cuerpo de `docs/release-notes-template.md`).
- [ ] **Paso 4:** el AppImage va con `continue-on-error: true`; si falla, el `.deb` basta (spec).
- [ ] **Paso 5:** commit `ci: flujo de release con borrador y SHA256SUMS`.

---

### Tarea 7: README y notas del *release*

**Archivos:** crear `README.md`, `docs/release-notes-template.md`.

**Interfaces:** la sección «Avisos de seguridad» de la spec, por sistema, con los textos exactos (SmartScreen: «Más información» → «Ejecutar de todas formas»; macOS: clic derecho → «Abrir» y `xattr -cr /Applications/Temiktli.app`; Linux: `chmod +x` / `sudo apt install ./Temiktli-….deb`; `sha256sum -c SHA256SUMS.txt`).

- [ ] **Paso 1:** escribir `README.md`: qué es, descarga (enlace a *Releases*), instalación por sistema, avisos, dónde se guardan los datos (las tres rutas), compilar desde el código, licencia MIT.
- [ ] **Paso 2:** escribir la plantilla de notas con la misma sección de avisos.
- [ ] **Paso 3:** verificar: `grep -c "xattr -cr" README.md docs/release-notes-template.md` da 1 en cada uno.
- [ ] **Paso 4:** commit `docs: README y notas de release`.

---

### Tarea 8: Primera versión de prueba y verificación

**Archivos:** ninguno (verificación y etiqueta).

- [ ] **Paso 1 (pedir confirmación, publica en el GitHub del usuario):** `git tag v0.1.0-rc.1 && git push origin v0.1.0-rc.1`.
- [ ] **Paso 2:** `gh run watch`; esperado: borrador de *Release* marcado *pre-release* con los instaladores de las tres plataformas y `SHA256SUMS.txt`. Si falla un trabajo, arreglar la causa y usar `-rc.2`.
- [ ] **Paso 3:** `sha256sum -c SHA256SUMS.txt` sobre lo descargado; `dpkg -I Temiktli-…deb | grep Depends` lista `libwebkit2gtk-4.1-0`.
- [ ] **Paso 4 (acción del usuario, sin automatizar):** instalar el `.exe` en Windows y abrir un HTML; comprobar que `%APPDATA%\Temiktli` existe; instalar otra vez encima y ver que `snippets.json` y `config.json` siguen ahí; desinstalar y ver que los datos se conservan.
- [ ] **Paso 5 (acción del usuario):** probar el `.dmg` y el `.deb`/AppImage en macOS y Linux y reportar qué falla; actualizar el README con lo que se descubra.
- [ ] **Paso 6:** con el visto bueno del usuario, publicar el borrador. La versión final `v0.1.0` es una etiqueta nueva tras poner `version = "0.1.0"` y repetir esta tarea.

---

## Autorrevisión frente a la spec

- Objetivos, repositorio y licencia → T1; carpeta de datos y mensaje de error → T2; fase previa de portabilidad y textos con «Windows» → T3; iconos → T4; formatos, identificador, NSIS por usuario, WebView2 y dependencias del `.deb` → T5; flujo, matriz, versionado, firma ad-hoc, `SHA256SUMS`, borrador y *pre-release* → T6; avisos de seguridad y README → T7; criterios de aceptación 1–7 → T3, T6 y T8.
- No-objetivos respetados: sin firma real, actualizador, tiendas ni ARM.
- Nombres consistentes entre tareas: `storage_path_in`, `check-version.sh`, `collect-artifacts.sh`, `icon.icns`, `icon-512.png`.
