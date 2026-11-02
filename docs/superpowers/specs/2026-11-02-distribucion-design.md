# Distribución de Temiktli — diseño

Fecha: 2026-11-02. Estado: **borrador para aprobación**. Quien lo lea debe poder hacer el plan sin volver a preguntar al autor; lo que decidió el usuario está marcado **[usuario]** y lo que decidió Claude, **[mi decisión]**.

## Intención

Hoy Temiktli solo se puede usar compilándola con `cargo` en la máquina del autor. El objetivo es que cualquier persona descargue un instalador desde GitHub, lo instale en su sistema y abra la aplicación sin tocar Rust ni la terminal (salvo el paso de macOS descrito más abajo).

## Objetivos

- **[usuario]** Instaladores para **Windows, macOS y Linux**.
- **[usuario]** Código **open source**, en el GitHub personal del autor; los instaladores se descargan de la página de *Releases* de ese repositorio.
- **[usuario]** Público desconocido: debe poder instalarla sin conocimientos técnicos.
- **[usuario]** Los instaladores **no se firman** en esta fase. Se acepta que los sistemas muestren avisos (ver «Avisos de seguridad»).
- **[usuario]** **Sin actualización automática**: para actualizar se descarga el instalador nuevo.
- Que sacar una versión sea una sola acción del autor: crear una etiqueta (`v0.1.0`) y esperar a que los instaladores aparezcan solos.
- Que cada instalador pueda reconstruirse desde la etiqueta, sin pasos manuales.

## No objetivos

- Firmar o notarizar ejecutables (queda como mejora futura; nada de este diseño la impide).
- Actualizador integrado, telemetría, informes de fallos.
- Publicar en tiendas (Microsoft Store, Mac App Store, Flathub, Snap), Homebrew, winget o AUR. Pueden añadirse después reutilizando los mismos artefactos.
- Soportar arquitecturas raras (Linux ARM, Windows ARM). **[mi decisión]**
- Reabrir el sistema de traducción: la interfaz ya está en siete idiomas y se cierra el tema.

## Anti-objetivos (fracaso aunque «funcione»)

- Un instalador que solo funciona en la máquina del autor (rutas absolutas, dependencias no declaradas).
- Instaladores publicados sin haber pasado los tests en las tres plataformas.
- Que la gente no sepa por qué su sistema le avisa y concluya que la aplicación es un virus: las instrucciones deben estar donde se descarga, no escondidas.
- Pérdida de datos del usuario al actualizar: el instalador nunca debe tocar `snippets.json` ni `config.json`.

## Estado actual (comprobado en el código)

- Workspace de Cargo con dos crates: `temiktli-core` (lógica) y `temiktli-shell` (ventana, con `wry` 0.57 + `tao` 0.37 + `rfd` + `trash`). Edición 2024. Versión `0.1.0`.
- **Todos los recursos van embebidos** (`include_str!` / `include_bytes!`): HTML, JS, iconos y los siete `locales/*.json`. El instalador solo tiene que llevar **un ejecutable**; no hay carpetas de recursos que empaquetar.
- `build.rs` ya incrusta `assets/icon.ico` en el `.exe` de Windows con `winresource`. Existen `icon.ico`, `icon.png` y `icon-64.rgba`; **no existe `.icns`** (macOS) ni icono de varios tamaños para Linux.
- Código casi portable: solo `i18n.rs` tiene `#[cfg(windows)]` y trae alternativa POSIX. `snippets.rs::storage_path()` usa `%APPDATA%` y, si no existe, `XDG_CONFIG_HOME` o `~/.config`: en Linux es correcto; **en macOS guardaría en `~/.config/Temiktli`** en vez de `~/Library/Application Support/Temiktli`.
- Algunos textos y comentarios dicen «el idioma de Windows»; hay que revisar que ninguna cadena visible al usuario nombre Windows en macOS/Linux.
- No es un repositorio git, no hay CI, licencia ni README. El directorio de trabajo contiene capturas, un vídeo y archivos de prueba (`content-large_groups.html`, etc.) que **no deben subirse**.
- El motor web embebido es distinto en cada sistema: WebView2 (Windows), WKWebView (macOS, viene con el sistema), WebKitGTK (Linux, hay que declararlo como dependencia).

## Decisiones de diseño (todas **[mi decisión]** salvo indicación)

### 1. Repositorio y licencia
- Inicializar git, añadir `.gitignore` (`target/`, capturas, vídeos, `content-large_groups*`, `handoff-*.md`) y subir a un repositorio **público** en la cuenta del autor.
- **[usuario]** Licencia: **MIT**, con un archivo `LICENSE` y el campo `license = "MIT"` en ambos `Cargo.toml`.
- **[usuario]** Usuario de GitHub: `vgarcias`. **[mi decisión]** Nombre del repositorio: `temiktli` (URL `github.com/vgarcias/temiktli`); el autor puede cambiarlo antes del primer *release*.
- `README.md`: qué es, capturas, descarga, instrucciones de instalación por sistema, compilar desde el código.

### 2. Herramienta de empaquetado: `cargo-packager`
Genera desde un único archivo de configuración: **NSIS** (`.exe`) en Windows, **`.app` + `.dmg`** en macOS, **`.deb` + AppImage** en Linux, a partir de un binario de Rust. Es el mismo ecosistema que `wry`/`tao`. Alternativas descartadas: `cargo-dist` (pensado para herramientas de línea de comandos, no apps de ventana), `cargo-bundle` (poco mantenido), Tauri completo (obligaría a reestructurar la aplicación).
- Configuración en `crates/temiktli-shell/Cargo.toml` (`[package.metadata.packager]`) o `Packager.toml`: nombre de producto «Temiktli», identificador `io.github.vgarcias.temiktli`, categoría de utilidades/desarrollo, descripción corta, iconos, licencia.
- Si `cargo-packager` falla en alguna plataforma de forma que no se arregle con configuración, el builder puede sustituir **solo esa plataforma** por una herramienta nativa (WiX/Inno Setup, `create-dmg`, `cargo-deb` + `appimagetool`), manteniendo los mismos nombres de archivo y el mismo *release*.

### 3. Formatos y nombres de los artefactos
| Sistema | Archivo | Notas |
|---|---|---|
| Windows x64 | `Temiktli-<ver>-windows-x64-setup.exe` (NSIS) | Instalador por usuario, sin pedir administrador. |
| macOS Apple Silicon | `Temiktli-<ver>-macos-arm64.dmg` | |
| macOS Intel | `Temiktli-<ver>-macos-x64.dmg` | Se compila con `--target x86_64-apple-darwin` desde el runner Apple Silicon (los runners Intel se están retirando). |
| Linux x64 | `Temiktli-<ver>-linux-x64.AppImage` y `Temiktli-<ver>-linux-x64.deb` | El `.deb` declara `libwebkit2gtk-4.1-0` y `libgtk-3-0` como dependencias. |

Además se publica `SHA256SUMS.txt` con las sumas de todos los artefactos.

### 4. Integración continua en GitHub Actions
- **`ci.yml`** — en cada *push* y *pull request*: `cargo test --workspace` y los 4 tests JS (`node crates/temiktli-shell/assets/<nombre>.test.js`, cada uno por separado) en **Windows, macOS y Linux**; `cargo build` sin warnings. Es lo primero que verifica la portabilidad real.
- **`release.yml`** — al crear una etiqueta `v*.*.*`: ejecuta los tests, construye los instaladores en una matriz (`windows-latest`, `macos-latest` más el objetivo Intel, `ubuntu-22.04`), los sube a un borrador de *Release* y adjunta `SHA256SUMS.txt`. El autor revisa el borrador y pulsa «Publicar»; publicar sigue siendo una acción humana.
- En Linux el flujo instala `libwebkit2gtk-4.1-dev`, `libgtk-3-dev` y lo necesario para AppImage. Se usa `ubuntu-22.04` para que los paquetes funcionen en distribuciones no demasiado recientes.
- La etiqueta y el campo `version` de `Cargo.toml` deben coincidir; el flujo falla si no.

### 5. Cambios de código necesarios
1. **Carpeta de datos por sistema**: sustituir la búsqueda manual de `APPDATA`/`XDG_CONFIG_HOME`/`HOME` por la carpeta estándar de cada sistema (crate `dirs` o equivalente): `%APPDATA%\Temiktli` en Windows (sin cambios, para no perder datos de quien ya la use), `~/Library/Application Support/Temiktli` en macOS, `$XDG_CONFIG_HOME` o `~/.config/Temiktli` en Linux. Con tests. El mensaje `status.language_save_failed` ya no debe fijar el texto «APPDATA».
2. **Iconos**: generar `icon.icns` (macOS) y un `icon.png` de al menos 512×512 (Linux) a partir de `logo.jpg`/`icon.png`, en `assets/`.
3. **Revisión de portabilidad**: `rfd` (diálogos de archivo), `trash` (papelera), menús y arrastre de ventana en macOS y Linux. Lo que no se pueda verificar sin esas máquinas se documenta como «no verificado» y se apoya en el CI; las comprobaciones visuales las hace el autor con los instaladores del borrador (no se automatiza el escritorio).
4. **Textos que nombran Windows** se hacen neutros. Cualquier cambio de texto va por claves en los siete JSON (los tests lo exigen).
5. Versión y metadatos (`authors`, `repository`, `description`, `license`, `homepage`) en los `Cargo.toml`.

### 6. Avisos de seguridad (consecuencia de **[usuario]** «sin firmar»)
Documentar en el `README` y en el texto de cada *Release*, con pasos claros:
- **Windows**: SmartScreen dice «Windows protegió su PC» → «Más información» → «Ejecutar de todas formas». En Windows 10 sin WebView2 el instalador NSIS debe instalar el *runtime* si falta (Windows 11 ya lo trae).
- **macOS**: el sistema dirá que la app está dañada o no se puede verificar. Instrucciones: arrastrar a Aplicaciones, clic derecho → «Abrir»; si no basta, `xattr -cr /Applications/Temiktli.app`. En Apple Silicon el builder debe aplicar **firma ad-hoc** (`codesign --sign -`, gratuita y sin cuenta de Apple) para que el `.app` pueda ejecutarse tras quitar la cuarentena. Esto no elimina el aviso.
- **Linux**: `chmod +x` al AppImage o instalar el `.deb` con `apt`. Sin avisos.
- `SHA256SUMS.txt` para quien quiera verificar la descarga.

### 7. Desinstalación y datos del usuario
- Los instaladores borran solo el programa. Los datos (`snippets.json`, `config.json`) **se conservan** al desinstalar y al actualizar. Se documenta dónde están.
- Actualizar = ejecutar el instalador nuevo encima (Windows, `.deb`) o reemplazar la app (macOS). Mismo identificador y misma carpeta de instalación para que se sobrescriba y no se duplique.

### 8. Aclaraciones tras la revisión (todas **[mi decisión]**)
- **Identificador y repositorio**: el usuario y el nombre del repositorio van en **un único sitio** de configuración (el del empaquetador) y de ahí se derivan README y notas del *release*. El identificador no puede cambiar después del primer *release* público, porque es lo que hace que una versión nueva sobrescriba a la anterior. Hay que fijarlo antes de `v0.1.0-rc.1`.
- **Windows sin administrador y WebView2**: instalador NSIS por usuario. Si falta WebView2 (Windows 10 antiguo), el instalador usa el *bootstrapper* pequeño de Microsoft (necesita internet solo en ese caso), que se instala por usuario si no hay permisos de administrador. No se incrusta el instalador offline (~120 MB). Si en la práctica pidiera elevación, se pide solo en ese caso.
- **Datos**: la carpeta de Windows no cambia (`dirs::config_dir()` devuelve `AppData\Roaming`, el mismo sitio; un test lo comprueba). En macOS nadie ha ejecutado la aplicación todavía, así que no hay datos antiguos en `~/.config` y **no se migra nada**.
- **Compilar y pasar tests en macOS/Linux es una fase previa**, antes de empaquetar. Se arreglan las causas, sin `#[allow]` ni recortes de `cfg` para silenciar avisos salvo que el aviso sea de verdad inaplicable en esa plataforma (y entonces con un comentario). Hasta que el CI esté en verde en las tres, no se toca el empaquetado.
- **Firma ad-hoc en macOS**: se aplica a las dos arquitecturas, sobre el `.app` **antes** de crear el `.dmg`. Si `cargo-packager` no lo permite (identidad de firma «-»), el trabajo de macOS construye el `.app` con el empaquetador, lo firma con `codesign` y crea el `.dmg` con `hdiutil` o `create-dmg`.
- **Tests JS**: se descubren con un patrón `assets/*.test.js`; cada uno se ejecuta por separado.
- **Versiones preliminares**: el `Cargo.toml` lleva la versión completa de SemVer (`0.1.0-rc.1`), la etiqueta es `v` + esa versión, y la comprobación compara ambas. Todo *release* cuya versión tenga guion se marca como *pre-release* automáticamente. En el `.deb` la versión preliminar se traduce con `~` (`0.1.0~rc.1`) para que `0.1.0` final se considere más nueva.
- **Linux mínimo**: basta el `.deb` para dar por cumplido el criterio de Linux; el AppImage es un extra si funciona.

## Riesgos conocidos

- **AppImage con WebKitGTK** es la parte más frágil. Si no se consigue uno fiable, se publica solo el `.deb` y se documenta; no se bloquea el resto.
- **macOS sin firma** puede ser más molesto de lo previsto en versiones recientes; el README debe estar al día con la versión de macOS que el autor pueda probar.
- Nadie ha ejecutado el programa en macOS o Linux todavía: el primer *release* puede revelar fallos que los tests no ven. Se mitiga publicando primero un *pre-release* `v0.1.0-rc1`.

## Criterios de aceptación

1. Una etiqueta `v0.1.0-rc1` produce, sin pasos manuales, los instaladores de las tres plataformas y `SHA256SUMS.txt` en un borrador de *Release*.
2. El CI pasa todos los tests en Windows, macOS y Linux.
3. En Windows, instalar con el `.exe` en una máquina limpia y abrir la aplicación funciona; los datos van a `%APPDATA%\Temiktli`.
4. En macOS y Linux, el autor (o quien tenga la máquina) instala, abre la aplicación y abre/guarda un archivo HTML; los datos van a la carpeta estándar del sistema.
5. Instalar una versión nueva encima conserva `snippets.json` y `config.json`.
6. El `README` explica en cada sistema cómo saltar el aviso de seguridad.
7. Ningún archivo ajeno (capturas, vídeo, `handoff-*`) está en el repositorio.

## Qué queda al criterio de quien lo construya

- Orden e idioma principal del `README`.
- Configuración fina de `cargo-packager`, nombres de flujos y *jobs*.
- Si el AppImage se publica o se omite (ver riesgos).
- Cómo generar los iconos mientras queden los formatos pedidos.

## Cosas que el autor debe hacer personalmente

- Crear el repositorio `vgarcias/temiktli` en GitHub (o autorizar que se haga con `gh`).
- Probar los instaladores de macOS y Linux del borrador de *Release* y decir qué falla.
