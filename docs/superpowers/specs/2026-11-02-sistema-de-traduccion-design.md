# Sistema de traducción de la interfaz — propuesta de diseño

Fecha: 2026-11-02. Aplicación: Temiktli (`D:\otros\rea`). Estado: **implementada** (módulo `i18n` en el núcleo, siete idiomas, detección del sistema, menú Vista → Idioma). Falta solo revisión humana de las traducciones y verificar en la ventana real. Lo único hecho son los archivos de idioma en `crates/temiktli-shell/locales/`: el inventario en español (`es.json`) y sus traducciones iniciales a inglés, francés, portugués, alemán, italiano y polaco.

## Intención y porqué

Toda la interfaz está escrita en español directamente en el código: en el HTML y el JavaScript de los webviews (`assets/*.html`, `editor-inject.js`), en los mensajes de la barra de estado y los diálogos de Rust (`main.rs`, `site.rs`, `fileops.rs`, `snippets.rs`) y en los textos del núcleo (`lint.rs`, `tags.rs`, `dom.rs`, `source.rs`, `command.rs`, `clean.rs`). El usuario quiere que la aplicación hable **el idioma del sistema** cuando lo tenga, **inglés** si no, y que el español quede como un idioma más, elegible. Para eso hace falta sacar los textos del código a archivos de idioma y un mecanismo que los busque en tiempo de ejecución.

## Objetivos

- Arrancar en el idioma de Windows si Temiktli lo tiene; si no, en inglés. Siete idiomas desde el principio: inglés, español, francés, portugués, alemán, italiano y polaco, todos con la misma cobertura.
- Un único archivo por idioma, fácil de completar por alguien que no programa.
- Que no se pueda olvidar una cadena: tests que comprueban que los archivos de idioma tienen las mismas claves y que el código no conserva literales en español.
- Sin dependencias nuevas: `serde_json` ya está en los dos crates.

## No objetivos

- Traducir el contenido de las páginas del usuario, ni detectar en qué idioma están.
- Pluralización completa (CLDR, géneros, casos). Basta `_one` / `_other`.
- Traducciones en caliente desde una carpeta externa: los idiomas van incrustados en el ejecutable.

## Antiobjetivos

- Un marcador `{nombre}` sin sustituir visible en pantalla.
- Que falte una clave en un idioma y la interfaz muestre la clave cruda: siempre se cae al inglés.
- Que el núcleo (`temiktli-core`) dependa de la interfaz o del sistema de archivos para formar un mensaje.

## Propuesta

### Archivos de idioma

- `crates/temiktli-shell/locales/{en,es,fr,pt,de,it,pl}.json`, ya escritos. `en.json` es el respaldo. `es.json` es el original (lo que la interfaz dice hoy); los otros seis son traducciones iniciales hechas desde él y **pendientes de revisión por alguien que hable cada idioma**, sobre todo el polaco.
- JSON anidado por zona de la interfaz. La clave que usa el código es la ruta con puntos: `menu.file.open`, `status.saved`, `lint.img_alt`.
- Marcadores con llaves y nombre: `"Guardado: {path}"`. Nunca por posición.
- Plurales con dos claves: `units.file_one` = `"{n} archivo"`, `units.file_other` = `"{n} archivos"`. Quien pide la cadena pasa `n` y el módulo elige.
- Listas de palabras (p. ej. `lint.vague_link_words`) como arrays: se unen las de todos los idiomas, porque miran el contenido de la página, no la interfaz.
- Bloque `$meta` con el código del idioma, su nombre para el selector y el separador decimal (`size_summary` escribe «2,0 KB» en español y «2.0 KB» en inglés).
- Los textos que acaban **dentro del documento del usuario** (elementos nuevos, snippets de serie, plantilla de archivo nuevo, `lang` del documento en blanco, el `track` con `srclang`) van en el bloque `content`. Ver la decisión abierta más abajo.

### Módulo `i18n` (en `temiktli-core`, compartido por los dos crates)

```rust
// temiktli-core/src/i18n.rs
pub struct Dict { /* HashMap<String, Value> aplanado con claves "a.b.c" */ }
static ACTIVE: OnceLock<RwLock<Dict>>;           // el idioma en uso
static FALLBACK: LazyLock<Dict>;                 // en.json, incrustado con include_str!

pub fn set_language(json: &str) -> Result<(), String>;   // lo llama la shell al arrancar
pub fn t(key: &str) -> String;                            // sin argumentos
pub fn tr(key: &str, args: &[(&str, &dyn Display)]) -> String;  // con {marcadores}
pub fn tn(key: &str, n: usize) -> String;                 // elige _one / _other y sustituye {n}
pub fn list(key: &str) -> Vec<String>;                    // arrays
pub fn language() -> String;                              // "en", "es"…
```

- Si la clave no está en el idioma activo se busca en `FALLBACK`; si tampoco, se devuelve la clave entre corchetes (`[status.saved]`) y un test lo detecta, nunca el usuario.
- Un macro `t!("status.saved", path = p.display())` para que las llamadas queden cortas.
- Los siete archivos JSON se incrustan en `temiktli-core` para que el núcleo pueda traducir sin que la shell le pase nada. La shell solo elige cuál activar. Una tabla `LANGUAGES: &[(&str, &str)]` (código, JSON) es la única lista de idiomas: de ella salen el selector del menú y la detección.
- En los tests el idioma activo es el inglés (el respaldo). Los tests que hoy comprueban cadenas en español deben pasar a comprobar la clave o el texto inglés; están listados al final.

### Cómo llega a cada capa

**Rust, shell y núcleo**: cada literal se sustituye por su clave. Los mensajes de error que el núcleo devuelve como `Result<_, String>` se traducen al construirse (el núcleo tiene el diccionario); la shell solo les antepone el `⚠`.

**Webviews** (`toolbar.html`, `popup.html`, `panel.html`, `code.html`, `editor-inject.js`):
- Rust ya sustituye marcadores (`/*TAGS_JSON*/`, `/*FIND_JS*/`). Se añade uno más, `/*I18N_JSON*/{}`, que recibe el diccionario del idioma activo **ya con el respaldo aplicado** (un solo objeto plano, clave → texto).
- Un `i18n.js` común (como `find.js`) define `t(key, args)` y `tn(key, n)`, y al cargar recorre `[data-i18n]`, `[data-i18n-title]` y `[data-i18n-placeholder]` para rellenar el marcado estático. Así el HTML queda `<button data-i18n="menu.file.open"></button>` y nada de texto en los archivos.
- Las cadenas que el JS compone (`'Última revisión: ' + …`, `' de '`, `'Sin resultados'`) pasan a `t(...)`.
- El `<html lang>` de cada webview toma `$meta.language`.
- El lienzo (`editor-inject.js`) solo tiene dos textos; recibe el diccionario por el mismo marcador.

### Elección del idioma

- Orden: argumento `--lang es` → variable `TEMIKTLI_LANG` → `config.json` (junto a `snippets.json`, en `%APPDATA%\Temiktli`) → **idioma del sistema** → `en`.
- **Detección del idioma del sistema**, sin crates nuevos: en Windows se declara a mano `extern "system" fn GetUserDefaultUILanguage() -> u16` de `kernel32` (el idioma en que el usuario ve Windows, no la configuración regional) y se mira el idioma primario (`langid & 0x3ff`: `0x09` en, `0x0a` es, `0x0c` fr, `0x16` pt, `0x07` de, `0x10` it, `0x15` pl). Fuera de Windows se leen `LC_ALL`, `LC_MESSAGES` y `LANG` y se toma el subtag primario (`pt_BR.UTF-8` → `pt`). Si el resultado no está en `LANGUAGES`, se usa `en`. Solo se mira la lengua, no la región: `es-MX` y `es-ES` comparten `es.json`.
- La detección se hace una vez al arrancar y no se escribe en `config.json`: así, si el usuario cambia el idioma de Windows, Temiktli lo sigue. Solo elegir en el menú lo fija.
- Un submenú **Vista → Idioma** (`Language`) con una entrada «Igual que el sistema» más los idiomas de `LANGUAGES`, cada uno escrito en su propia lengua (`$meta.name`), que escribe `config.json`. Primera versión: avisa en la barra de estado de que se aplica al reiniciar. El diseño permite cambiarlo en caliente (volver a inyectar el diccionario y recorrer los `data-i18n`), pero no es necesario para empezar.
- Test de la detección: la función que convierte un LANGID o una variable de entorno en código de idioma es pura y se prueba con tablas (`0x0c0a` → `es`, `0x0416` → `pt`, `"pt_BR.UTF-8"` → `pt`, `"C"` → `en`, `0x0411` (japonés) → `en`).

### Qué no se traduce

- Nombres de etiquetas y atributos HTML, propiedades CSS, rutas, extensiones, nombres de archivo del usuario.
- Mensajes de `eprintln!` y `cargo:warning` (son para quien desarrolla).
- Símbolos (`⚠`, `●`, `▸`, `✔`): van dentro de la cadena traducida para que cada idioma decida.

### Verificación (sin abrir la ventana)

- Test en `temiktli-core`: los siete archivos tienen exactamente el mismo conjunto de claves que `en.json`, los marcadores `{x}` de cada clave coinciden entre idiomas, y cada `_one` tiene su `_other`.
- Test: todas las claves que aparecen en el código (`t!("…")` en Rust, `t('…')` y `data-i18n="…"` en los assets) existen en `en.json`. Se obtiene con una expresión regular sobre `include_str!` de los fuentes, como ya hacen los tests de cableado de los popups.
- Test: ningún fuente (fuera de `#[cfg(test)]` y de los `locales/`) contiene literales con `á é í ó ú ñ ¿ ¡ «`. Es un detector de olvidos, no una prueba de corrección.
- Edge headless con las páginas ensambladas (como hasta ahora) para comprobar que ningún `data-i18n` queda vacío y ningún `[clave]` entre corchetes aparece en la captura.

## Plan de trabajo (cada paso con tests antes de implementar)

- Módulo `i18n` en el núcleo con los siete JSON incrustados; tests de claves iguales y de respaldo.
- Revisar los términos de `en.json` con el usuario (p. ej. «Snippets», «Lienzo» → «Canvas») y pedir revisión de los otros cinco a hablantes nativos cuando se pueda.
- Sustituir literales en `temiktli-core` (`lint`, `tags`, `dom`, `source`, `command`, `clean`); adaptar sus tests.
- Sustituir literales en `temiktli-shell` (`main`, `site`, `fileops`, `snippets`); adaptar sus tests.
- `i18n.js` y marcado `data-i18n` en los cuatro webviews y el lienzo; marcador `/*I18N_JSON*/`; pruebas en Edge headless.
- Detección del idioma del sistema (función pura con tests de tabla), `config.json`, `--lang`, `TEMIKTLI_LANG` y submenú Vista → Idioma.
- Test detector de literales en español y limpieza final.

## Riesgos y detalles conocidos

- **Tests que afirman español** y habrá que tocar: en `main.rs` los de `delete_summary`, `rename_summary`, `replace_text`, `unchecked_text` (buscan «Papelera», «Sí», «Cancelar»…), `save_status`, `NEW_FILE_NAME == "nuevo.html"`, el de `TAGS_JSON` que busca «Énfasis y estilo»; en `lint.rs` los que buscan trozos de mensaje; en `snippets.rs` los nombres de serie; en `site.rs` las descripciones de los casos (solo comentarios de test, no cambian).
- **Nombre del archivo nuevo**: hoy `nuevo.html`; cada idioma trae el suyo en `app.new_file_name` (`new.html`, `nouveau.html`…). Hay un test que fija el nombre.
- **Plurales**: `_one` / `_other` basta para los siete idiomas elegidos salvo el polaco, que distingue 2-4 (`pliki`) de 5+ (`plików`). Primera versión: `_other` lleva la forma de 5+ (correcta para la mayoría de los números) y se anota como mejora pendiente añadir `_few` solo cuando `$meta` lo declare.
- **Longitud de los textos**: alemán, francés y polaco son más largos que el español; los menús se miden solos (`MenuSize`) pero la barra de menús tiene ancho fijo y puede necesitar que «Buscar archivos» e «Insertar etiquetas» se acorten.
- **El arreglo de un clic «Añadir lang="es"»** pone `es` fijo. Propuesta: usar el idioma del documento (`content.document_lang`), que en inglés sería `en`.
- **Decimal**: `size_summary` cambia el punto por coma a mano; pasa a leer `$meta.decimal_separator`.
- **Contenido del documento** (bloque `content`): si se mantiene atado al idioma de la interfaz, un usuario con interfaz en inglés que escribe páginas en español obtendrá elementos nuevos con «Heading» y snippets con «Site name». Alternativa: un ajuste aparte «idioma de las páginas» que solo gobierne `content`. Lo decide el usuario; la propuesta inicial es atarlo al de la interfaz por simplicidad.
- **Teclas**: `Mayús` y `Supr` son nombres en español; en inglés `Shift` y `Del`. Van en `keys`.
- **Orden de palabras**: cadenas compuestas por concatenación (p. ej. `'Última revisión: ' + lista`) se reescriben como una sola clave con marcadores, para que otro idioma pueda reordenar.
- **Tamaño**: incrustar siete JSON de unos 30 KB no cambia nada apreciable en el ejecutable ni en el arranque.

## Decisiones que quedan al usuario

- Si el bloque `content` sigue al idioma de la interfaz o tiene su propio ajuste.
- Cambio de idioma al reiniciar (primera versión) o en caliente.
- Quién revisa las traducciones de francés, portugués, alemán, italiano y polaco antes de publicarlas.
- Términos en inglés para palabras propias del programa: «Snippets», «Lienzo», «Revisión», «Reducir código».

## Inventario entregado

`crates/temiktli-shell/locales/` contiene `es.json` (original), `en.json`, `fr.json`, `pt.json`, `de.json`, `it.json` y `pl.json`, todos con las mismas 541 claves. Cada uno recoge, por bloques: `app`, `keys`, `toolbar`, `menu` (archivo, edición, vista, buscar, revisión, snippets, etiquetas), `panel` (insertar, propiedades, archivos, estilos), `code` (editor y buscar/reemplazar), `canvas`, `units` (plurales y tamaños), `dialog` (los cinco diálogos de rfd), `status` (toda la barra de estado), `snippets`, `site`, `command`, `nesting` (reglas de anidación de `tags.rs`), `source`, `clean`, `lint` (mensajes, pistas de obsoletos, arreglos, listas de palabras) y `content`. Se dejaron fuera a propósito los mensajes de desarrollo (`eprintln!`, `build.rs`) y los textos de los tests.
