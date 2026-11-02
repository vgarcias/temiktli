# Renombrar y borrar archivos y carpetas del sitio — Plan de implementación

> **Para quien lo ejecute:** usar superpowers:subagent-driven-development (recomendado) o superpowers:executing-plans, tarea por tarea. Los pasos usan casillas `- [ ]`.

**Objetivo:** renombrar y borrar archivos y carpetas del sitio desde el panel Archivos, avisando de los enlaces de otros HTML/CSS que quedarían rotos y dejando decidir al usuario.

**Arquitectura:** dos módulos nuevos en `temiktli-shell` con lógica pura y testeada (`relink.rs`: encontrar y reescribir enlaces; `fileops.rs`: impacto, plan de renombrado, aplicar en disco, Papelera detrás de un trait). `main.rs` solo pega: mensajes del panel → diálogos `rfd` → llamadas a esos módulos → actualizar buffers abiertos. El panel (`panel.html`) añade menú contextual, campo de renombrado en línea y atajos F2/Supr.

**Tecnología:** Rust 2024, `rfd` 0.17 (ya en uso), crate nueva `trash` (última 5.x), JS sin dependencias en `panel.html`.

**Spec:** `docs/superpowers/specs/2026-11-02-renombrar-borrar-archivos-design.md`

## Restricciones globales

- Idioma de toda la interfaz y mensajes: español.
- Enlaces que cuentan: `href` y `src` en `.html`/`.htm`; `url(...)` y `@import` en `.css`. Nada más.
- Borrar va a la **Papelera de Windows**; si falla, **nunca** se borra de forma definitiva, solo se avisa. Sin `.bak` en estas operaciones.
- Borrar: un solo botón de confirmar, con detalle (archivos, subcarpetas, enlaces rotos y qué archivos). Renombrar con enlaces afectados: botones «Renombrar y actualizar los enlaces» / «Renombrar sin tocar los enlaces» / «Cancelar».
- Nombre nuevo ya existente: preguntar si reemplazar (el existente a la Papelera), vale cualquier combinación archivo/carpeta. Cambio de extensión: confirmación extra. Renombrar no se deshace con Ctrl+Z.
- Documento abierto: renombrar → sigue abierto con la ruta nueva; borrar → sigue abierto sin ruta (`path = None`, `dirty = true`) y Guardar actúa como «Guardar como…».
- Solo dentro de la raíz del sitio. Ocultos y enlaces simbólicos se omiten al recorrer.
- Cambiar solo el valor del enlace; no reformatear el resto del archivo.
- Entorno: antes de `cargo`, `$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"`. Prueba: `cargo test --workspace 2>&1 | Select-String -Pattern "^(error|warning)|test result|FAILED|panicked"` (hoy: 162 pasan, sin warnings). No es un repositorio git: no hay pasos de commit; el punto de control de cada tarea es la suite completa en verde.
- Nunca automatizar ratón/teclado ni lanzar la GUI. Lo visual se verifica con Edge headless (`--headless=new --allow-file-access-from-files --screenshot=…`; `--dump-dom` no imprime: volcar el resultado en un `<pre>` visible y leer la captura) con `window.ipc` simulado.
- Hook GateGuard: la primera edición/creación de cada archivo y el primer PowerShell de la sesión piden exponer antes en texto quién lo importa, qué funciones afecta, esquema de datos y la instrucción del usuario. Ediciones en paralelo: comprobar cada resultado y rehacer las rechazadas.

## Foco de revisión (casos que el diseño implica y ninguna tarea nombraba)

1. Renombrar una carpeta: los enlaces **absolutos del sitio** (`/a/x.png`) dentro de sus propios archivos se actualizan; los relativos siguen válidos. (Test en T2.)
2. Coincidencia por **segmentos**, no por prefijo: renombrar `img` no toca enlaces a `img2/a.png` ni `images/`. (T1.)
3. Cambiar solo mayúsculas (`a.css` → `A.css`) funciona en Windows y no cuenta como «ya existe». (T4.)
4. El enlace conserva su estilo: comillas dobles/simples/sin comillas, `?v=2#x`, `%20` y, si era `/…`, sigue siendo absoluto. (T2.)
5. Archivos que no se pueden revisar (no UTF-8, ilegibles) no se ignoran en silencio: el aviso dice cuántos no se revisaron. (T3.)

---

### Tarea 1: Encontrar enlaces (`relink.rs`, parte pura)

**Archivos:**
- Crear: `crates/temiktli-shell/src/relink.rs` (con `#[cfg(test)] mod tests`)
- Modificar: `crates/temiktli-shell/src/main.rs` (solo `mod relink;`)

**Interfaces:**
- Produce:
  - `pub enum Kind { Html, Css }` y `pub fn kind_of(rel: &str) -> Option<Kind>` (`.html`/`.htm` → Html, `.css` → Css, sin distinguir mayúsculas; otro → `None`).
  - `pub struct LinkRef { pub start: usize, pub end: usize, pub value: String }` — rango en bytes del **valor** del enlace dentro del texto (sin comillas).
  - `pub fn scan(text: &str, kind: Kind) -> Vec<LinkRef>` — en Html, atributos `href`/`src` (comillas dobles, simples o sin comillas; nombre sin distinguir mayúsculas, solo dentro de etiquetas); en Css, `url(...)` (con o sin comillas) y `@import "x"`/`@import 'x'`.
  - `pub fn resolve(from_rel: &str, value: &str) -> Option<String>` — ruta del sitio a la que apunta `value` visto desde el archivo `from_rel`: relativa a su carpeta, o desde la raíz si empieza por `/`; admite `..`; quita `?consulta`/`#ancla`; decodifica `%XX`. `None` para URL con esquema (`https:`, `mailto:`, `data:`…), `//host`, solo `#ancla`, vacío, o si sube por encima de la raíz.
  - `pub fn points_into(resolved: &str, target: &str, is_dir: bool) -> bool` — igual (sin mayúsculas) a `target`, o, si `is_dir`, dentro de él por **segmentos**.

- [ ] **Paso 1: tests que fallan** (nombres y aserciones):
  - `scan_finds_href_and_src_in_all_quote_styles`: sobre `<a href="a.html"><img src='img/b.png'><link href=c.css>` devuelve valores `a.html`, `img/b.png`, `c.css` y `&text[r.start..r.end] == r.value` para cada uno; ignora `data-href="x"` y texto fuera de etiquetas.
  - `scan_css_finds_url_and_import`: `@import "base.css"; a{background:url( 'img/x.png' )} b{background:url(y.png)}` → `base.css`, `img/x.png`, `y.png`.
  - `resolve_is_relative_to_the_containing_file`: `resolve("blog/post.html","../css/a.css") == Some("css/a.css")`; `resolve("blog/post.html","img/a.png") == Some("blog/img/a.png")`; `resolve("blog/post.html","/css/a.css") == Some("css/a.css")`.
  - `resolve_strips_suffix_and_decodes`: `resolve("i.html","mi%20foto.png?v=2#x") == Some("mi foto.png")`.
  - `resolve_rejects_non_site_links`: `None` para `https://x.com/a`, `//cdn/a.css`, `mailto:a@b`, `data:text/css,p{}`, `#top`, `""`, `../../fuera.css` desde `i.html`.
  - `points_into_matches_whole_segments`: `points_into("img/a.png","img",true)`; no `points_into("img2/a.png","img",true)`; `points_into("IMG/a.png","img",true)`; archivo exacto con `is_dir=false`; `!points_into("img/a.png","img",false)`.
- [ ] **Paso 2:** `cargo test -p temiktli-shell relink` → falla (no compila / no definido).
- [ ] **Paso 3:** implementar lo de Interfaces en `relink.rs`. Escáner a mano sobre `&str` (sin crate de HTML): en Html, recorrer etiquetas `<…>` y dentro de ellas buscar nombres de atributo con límite de palabra.
- [ ] **Paso 4:** `cargo test -p temiktli-shell relink` → pasan.

### Tarea 2: Reescribir enlaces (`relink.rs`, parte pura)

**Archivos:**
- Modificar: `crates/temiktli-shell/src/relink.rs`

**Interfaces:**
- Consume: `scan`, `resolve`, `points_into`, `Kind` (T1).
- Produce:
  - `pub fn retarget(text: &str, kind: Kind, from_old: &str, from_new: &str, old: &str, new: &str, is_dir: bool) -> (String, usize)` — reescribe cada enlace que, **resuelto desde `from_old`**, apunta a `old` (o dentro, si `is_dir`), de modo que **escrito desde `from_new`** apunte a `new` (o al mismo subcamino dentro de `new`). Devuelve el texto nuevo y cuántos cambió. `from_old`/`from_new` difieren solo cuando el propio archivo se mueve por renombrar su carpeta. Conserva el estilo: si el original empezaba por `/`, el nuevo también; si era relativo, relativo (con `../` si hace falta); conserva `?consulta`/`#ancla`; codifica espacios y `# ? % " < >` como `%XX` (igual que `site::image_src`).

- [ ] **Paso 1: tests que fallan:**
  - `retarget_renames_a_file_link_keeping_style_and_suffix`: `<link href='css/a.css?v=2#x'>` con old=`css/a.css`, new=`css/b.css` → `<link href='css/b.css?v=2#x'>`, cuenta 1; el resto del texto idéntico byte a byte.
  - `retarget_keeps_root_absolute_and_unquoted`: `href=/css/a.css` → `href=/css/b.css`.
  - `retarget_encodes_spaces`: new=`css/mi hoja.css` → `css/mi%20hoja.css`.
  - `retarget_directory_rewrites_inner_paths_by_segment`: old=`img`, new=`fotos`, is_dir; `<img src="img/a/b.png">` → `fotos/a/b.png`; `src="img2/x.png"` sin cambios.
  - `retarget_relative_from_a_subfolder`: archivo `blog/p.html`, `src="../img/a.png"`, old=`img/a.png`, new=`img/b.png` → `../img/b.png`.
  - `retarget_moves_with_its_own_folder`: carpeta `a` → `b`; el archivo `a/p.html` (`from_old="a/p.html"`, `from_new="b/p.html"`) con `src="/a/x.png"` → `/b/x.png` y con `src="x.png"` (relativo, apunta a `a/x.png`) queda `x.png`.
  - `retarget_css_url_and_import`: `url("img/a.png")` y `@import 'a.css'` se reescriben.
  - `retarget_ignores_external`: `https://x.com/img/a.png` no se toca.
- [ ] **Paso 2:** ver fallar. **Paso 3:** implementar `retarget` con los `LinkRef` de `scan` aplicados de atrás hacia delante. **Paso 4:** `cargo test -p temiktli-shell relink` → pasan.

### Tarea 3: Impacto en disco (`fileops.rs`)

**Archivos:**
- Crear: `crates/temiktli-shell/src/fileops.rs` (con tests, helper `site()` copiando el estilo de `site.rs` tests: carpeta temporal por prueba)
- Modificar: `crates/temiktli-shell/src/main.rs` (`mod fileops;`)

**Interfaces:**
- Consume: `relink::{scan, resolve, points_into, kind_of, Kind}`.
- Produce:
  - `pub struct Referrer { pub rel: String, pub links: usize }`
  - `pub struct Impact { pub referrers: Vec<Referrer>, pub not_checked: usize, pub files: usize, pub dirs: usize }`
  - `pub fn impact(root: &Path, target_rel: &str, is_dir: bool, ignore_inside: bool) -> Impact` — recorre el disco **sin los límites de `list_site`** (omite ocultos y enlaces simbólicos, sí incluye `.bak`), lee los `.html/.htm/.css` en UTF-8 (quita BOM), cuenta por archivo los enlaces que apuntan al destino (`points_into`). `ignore_inside = true` (borrar) no cuenta archivos que están dentro del propio destino. `files`/`dirs` = cuántos archivos y subcarpetas contiene el destino (0 y 0 si es archivo). `not_checked` = `.html/.htm/.css` no UTF-8 o ilegibles.

- [ ] **Paso 1: tests que fallan:**
  - `impact_lists_files_that_link_to_a_file`: `index.html` con `href="css/main.css"` y `blog/p.html` con `href="../css/main.css"` → dos referrers, `links == 1` cada uno, `not_checked == 0`.
  - `impact_of_a_folder_counts_inner_links_and_contents`: carpeta `img` con 2 archivos y 1 subcarpeta → `files == 2`, `dirs == 1`; enlaces a `img/a.png` e `img/sub/b.png` suman.
  - `delete_ignores_links_from_inside_the_target`: con `ignore_inside = true` un `img/leeme.html` que enlaza a `img/a.png` no es referrer; con `false` sí.
  - `impact_reports_unreadable_files`: un `.html` con bytes inválidos UTF-8 → `not_checked == 1`.
  - `impact_skips_hidden_and_symlink_dirs`: un enlace desde `.git/x.html` no cuenta.
  - `impact_goes_deeper_than_the_panel_listing`: referrer a profundidad 6 se encuentra.
- [ ] **Paso 2:** ver fallar. **Paso 3:** implementar. **Paso 4:** `cargo test -p temiktli-shell fileops` → pasan.

### Tarea 4: Renombrar y borrar en disco (`fileops.rs`)

**Archivos:**
- Modificar: `crates/temiktli-shell/src/fileops.rs`, `crates/temiktli-shell/Cargo.toml` (`trash = "5"`; usar `cargo add trash -p temiktli-shell` para fijar la última 5.x)

**Interfaces:**
- Consume: `site::{rel_key, resolve_in_root}`, `site::check_segments` (hacerla `pub(crate)`), `relink::retarget`, `Impact` (T3).
- Produce:
  - `pub trait Trash { fn send(&self, abs: &Path) -> Result<(), String>; }` y `pub struct SystemTrash;` que lo implementa con la crate `trash` (`trash::delete`); en tests, un `FakeTrash` que mueve a una carpeta temporal.
  - `pub struct RenamePlan { pub from_rel: String, pub to_rel: String, pub from_abs: PathBuf, pub to_abs: PathBuf, pub is_dir: bool, pub unchanged: bool, pub case_only: bool, pub ext_changed: bool, pub exists: bool, pub exists_is_dir: bool }`
  - `pub fn plan_rename(root: &Path, rel: &str, new_name: &str) -> Result<RenamePlan, String>` — `new_name` es solo el nombre (sin `/`); valida con `check_segments`; `from` debe existir dentro de la raíz (archivo o carpeta, sin symlink); `unchanged` si `new_name == actual`; `case_only` si solo difieren mayúsculas; `exists` si el destino existe y **no** es el mismo elemento por cambio de mayúsculas; `ext_changed` compara extensiones en minúsculas (solo archivos).
  - `pub fn apply_rename(plan: &RenamePlan, replace: bool, trash: &dyn Trash) -> Result<(), String>` — si `exists` y `!replace` → error; si `exists && replace`, envía el existente a la Papelera y luego `fs::rename`; si falla la Papelera, no renombra.
  - `pub struct RelinkReport { pub updated: Vec<String>, pub failed: Vec<(String, String)> }`
  - `pub fn relink_site(root: &Path, plan: &RenamePlan, skip: &[String]) -> RelinkReport` — **después** de renombrar en disco, reescribe en disco los `.html/.htm/.css` con enlaces al elemento (con `retarget`, usando `from_old` = ruta antigua del archivo y `from_new` = la nueva si estaba dentro de la carpeta renombrada). `skip` = rutas (`rel_key`) de archivos abiertos con cambios en memoria, que el llamador trata aparte. Solo escribe si hubo cambios; conserva BOM y saltos de línea. Un fallo no detiene los demás.
  - `pub fn delete_to_trash(root: &Path, rel: &str, trash: &dyn Trash) -> Result<(), String>` — resuelve dentro de la raíz (archivo o carpeta) y llama a `trash.send`; el error se devuelve tal cual, nunca se borra por otra vía.

- [ ] **Paso 1: tests que fallan:**
  - `plan_rename_flags`: `plan_rename(root,"css/main.css","tema.css")` → `ext_changed == false`, `exists == false`, `to_rel == "css/tema.css"`; con `"tema.txt"` → `ext_changed`; con `"main.css"` → `unchanged`; con `"Main.css"` → `case_only && !exists`; con `"a.css"` existente → `exists`.
  - `plan_rename_rejects_bad_names`: `""`, `"a/b.css"`, `"a<b.css"`, `"con.css"`, `".."`, `"x."` → `Err`; `rel` inexistente o `../fuera` → `Err`.
  - `apply_rename_moves_a_file_and_a_folder`.
  - `apply_rename_case_only_works`: `a.css` → `A.css` con `replace = false` funciona.
  - `apply_rename_replace_sends_existing_to_trash`: el `FakeTrash` recibe el archivo existente y el renombrado ocupa su sitio; sin `replace` → `Err` y nada cambia; con `FakeTrash` que falla → `Err` y el origen intacto.
  - `apply_rename_replace_works_across_types`: carpeta sobre archivo y archivo sobre carpeta.
  - `relink_site_updates_other_files_on_disk`: tras renombrar `css/main.css` → `css/tema.css`, `index.html` y `blog/p.html` apuntan a `tema.css`; `updated` los lista; el resto del texto idéntico; un archivo en `skip` no se toca.
  - `relink_site_folder_rename_fixes_inner_absolute_links` (Foco 1).
  - `relink_site_reports_failures_and_continues`: un archivo de solo lectura aparece en `failed` y los demás se actualizan.
  - `delete_to_trash_calls_trash_and_never_removes`: con `FakeTrash` que falla el archivo sigue existiendo; `delete_to_trash(root,"../x",…)` → `Err`.
- [ ] **Paso 2:** ver fallar. **Paso 3:** implementar; `SystemTrash` sin test automático (usa la Papelera real). **Paso 4:** `cargo test -p temiktli-shell fileops` → pasan; `cargo test --workspace` sin warnings.

### Tarea 5: Mensajes y flujo de borrado en la aplicación

**Archivos:**
- Modificar: `crates/temiktli-core/src/protocol.rs` (variantes nuevas en `FromWebview`), `crates/temiktli-core/src/lib.rs` (test serde junto al de la línea ~505), `crates/temiktli-shell/src/main.rs` (despacho en `handle`, ~1359; método nuevo en `impl App`, junto a `new_file`, ~1669)

**Interfaces:**
- Consume: `fileops::{impact, delete_to_trash, SystemTrash}`, `App::site_root`, `App::refresh_files`, `App::css_index`, `App::sync_overrides`.
- Produce:
  - `FromWebview::DeletePath { path: String }` y `FromWebview::RenamePath { path: String, new_name: String }` (esta última se usa en T6).
  - `fn delete_path(&mut self, rel: &str)` en `App`.
  - función pura `fn delete_summary(rel: &str, is_dir: bool, impact: &fileops::Impact, dirty_css: usize) -> String` (texto del diálogo, testeable sin ventana).

- [ ] **Paso 1: tests que fallan:**
  - en `temiktli-core`: `{"type":"DeletePath","path":"img"}` y `{"type":"RenamePath","path":"a.css","new_name":"b.css"}` se deserializan.
  - en `main.rs` tests: `delete_summary` de archivo sin enlaces → menciona «Papelera» y no menciona enlaces; de carpeta con 3 archivos, 1 subcarpeta y 2 HTML que enlazan → contiene «3 archivos», «1 subcarpeta», los dos nombres de archivo y «enlaces»; con `not_checked = 2` → dice que 2 archivos no se pudieron revisar; con `dirty_css > 0` → avisa de las hojas abiertas sin guardar que se cerrarán.
- [ ] **Paso 2:** ver fallar. **Paso 3:** implementar.
  - `delete_path`: sin carpeta de sitio → estado de aviso; resolver `rel` (archivo o carpeta) dentro de la raíz; `impact(.., ignore_inside = true)`; `MessageDialog` con un único botón de confirmar (`OkCancelCustom` con «Borrar»/«Cancelar») y el texto de `delete_summary`; si confirma, `delete_to_trash(root, rel, &SystemTrash)`.
  - Tras borrar: cerrar los buffers `css_files` que están dentro del destino (si había alguno sin guardar, ya se avisó en el diálogo); si el documento abierto es el destino o está dentro: `self.path = None`, `self.dirty = true`, `sync_shared()`, `reload_canvas()`, `code_force = true`; `refresh_files()`; estado «Enviado a la Papelera: …».
  - Mi decisión (no está en el diseño): con `path = None` el panel Archivos deja de ver la carpeta del sitio («Guarda el documento para ver los archivos…») hasta que se guarde con «Guardar como…».
- [ ] **Paso 4:** `cargo test --workspace` → pasan, sin warnings.

### Tarea 6: Flujo de renombrado en la aplicación

**Archivos:**
- Modificar: `crates/temiktli-shell/src/main.rs`

**Interfaces:**
- Consume: `fileops::{plan_rename, apply_rename, relink_site, impact, SystemTrash, RenamePlan}`, `relink::{retarget, kind_of}`, el mecanismo de `link_css` (~685) para reemplazar el documento abierto con `ReplaceDocument`.
- Produce:
  - `fn rename_path(&mut self, rel: &str, new_name: &str)` en `App`.
  - función pura `fn rename_open_path(old_open: &str, old: &str, new: &str, is_dir: bool) -> Option<String>` — nueva ruta de un archivo abierto cuando se renombra `old`→`new` (él mismo o una carpeta que lo contiene); `None` si no le afecta. Comparación con `rel_key`.
  - función pura `fn rename_summary(plan: &fileops::RenamePlan, impact: &fileops::Impact) -> String`.

- [ ] **Paso 1: tests que fallan:** `rename_open_path("css/a.css","css/a.css","css/b.css",false) == Some("css/b.css")`; `rename_open_path("img/x/a.png","img","fotos",true) == Some("fotos/x/a.png")`; `rename_open_path("img2/a.png","img","fotos",true) == None`; `rename_summary` lista los archivos afectados y cuenta enlaces.
- [ ] **Paso 2:** ver fallar. **Paso 3:** implementar `rename_path` en este orden:
  1. `plan_rename`; `unchanged` → no hace nada; `Err` → aviso en la barra de estado.
  2. `ext_changed` → diálogo de confirmación extra («cambia el tipo de archivo»).
  3. `exists` → diálogo «¿Reemplazar?» (el existente va a la Papelera).
  4. `impact(.., ignore_inside = false)`; si hay referrers → diálogo de tres botones (`YesNoCancelCustom`) con `rename_summary`; si no, renombra directo.
  5. `apply_rename`; si falla, estado con el error y nada más.
  6. Si eligió actualizar: `relink_site` saltando los archivos abiertos con cambios en memoria; para el documento HTML abierto y los `css_files` abiertos, aplicar `retarget` al texto en memoria (documento vía `ReplaceDocument` como `link_css`, hojas editando `text`); informar de `failed`.
  7. Actualizar rutas abiertas con `rename_open_path`: `self.path` (la ruta absoluta se reconstruye desde la raíz), `css_files[i].rel/abs`, `active_file`, y `sync_shared`, `sync_overrides`, `refresh_canvas_css`, `refresh_files`, `reload_canvas`.
  8. Estado: «Renombrado: a → b» (+ «N enlaces actualizados» / «⚠ no se pudo actualizar: …»).
  - Si el destino reemplazado estaba abierto como hoja, se cierra su buffer.
- [ ] **Paso 4:** `cargo test --workspace` → pasan, sin warnings.

### Tarea 7: Panel — menú contextual, renombrado en línea y atajos

**Archivos:**
- Modificar: `crates/temiktli-shell/assets/panel.html` (`renderFiles`, ~144; estilos `.tree`, ~74; listeners, ~199)

**Interfaces:**
- Consume: mensajes `RenamePath { path, new_name }` y `DeletePath { path }` (T5).
- Produce: en cada fila de archivo o carpeta, evento `contextmenu` que abre un menú propio con «Renombrar» (F2) y «Borrar» (Supr); selección de fila (clase `.sel`, una a la vez); **F2** convierte el nombre en un `<input>` (valor inicial el nombre actual, sin extensión seleccionada): Enter envía `RenamePath` con solo el nombre, Esc o perder el foco cancela; **Supr** envía `DeletePath`. F2/Supr solo actúan con el foco dentro de `#files` y sin un `<input>` enfocado. El menú se cierra con Esc, clic fuera o tras elegir. `filesSig` debe incluir la ruta seleccionada y el modo de edición para que `renderFiles` no pise el `<input>`.

- [ ] **Paso 1:** implementar en `panel.html` (sin dependencias).
- [ ] **Paso 2: verificación con Edge headless** (no la ventana real): página temporal en el scratchpad que carga `panel.html` con `window.ipc` simulado (`postMessage` apila en un array) y un `<pre>` fijo con el resultado; eventos sintéticos: `contextmenu` sobre un archivo y una carpeta muestra el menú con los dos elementos; clic en «Renombrar» crea el `<input>`; Enter con `b.css` apila `{"type":"RenamePath","path":"css/a.css","new_name":"b.css"}`; Esc cancela sin enviar; F2 y Supr con la fila seleccionada envían lo esperado; F2 con foco en `#newfile` no hace nada. Captura con `--screenshot` y leer el `<pre>`. `node --check` del JS extraído.
- [ ] **Paso 3:** `cargo test --workspace` → pasan (incluye que el HTML se compile dentro del binario).

### Tarea 8: Cierre

- [ ] **Paso 1:** `cargo test --workspace` completo: todo en verde, sin warnings; anotar el número de tests.
- [ ] **Paso 2:** revisión con `ecc:rust-review` sobre `relink.rs`, `fileops.rs` y los cambios de `main.rs`; corregir lo grave.
- [ ] **Paso 3:** resumen al usuario separando lo verificado sin ventana (tests, Edge headless) de lo que solo él puede confirmar en su ventana: menú contextual, F2/Supr, diálogos nativos (tres botones, confirmaciones), envío real a la Papelera, que el documento abierto siga abierto tras renombrar/borrar y Guardar como tras borrar. Recordar los pendientes de ajuste (arrastre y cursor de inserción) y preguntar por `assets/sample.html`.
