# Renombrar y borrar archivos y carpetas del sitio — diseño

Fecha: 2026-11-02. Aplicación: Temiktli (`D:\otros\rea`). Idioma de la interfaz: español.

## Intención y porqué

El panel Archivos lista los archivos del sitio abierto pero no permite renombrarlos ni borrarlos. Quien edita un sitio tiene que salir a otro programa para ello, y al hacerlo se rompen enlaces sin darse cuenta. Queremos hacerlo desde el panel y que el programa avise de los enlaces de otros archivos que quedarían rotos, dejando la decisión al usuario. El usuario no tiene una preocupación concreta más allá de eso; lo que decidió es el comportamiento de abajo.

## Objetivos

- Renombrar y borrar archivos **y carpetas** del sitio desde el panel Archivos.
- Avisar, antes de actuar, de los enlaces de otros archivos afectados, y dejar que el usuario decida.
- Que borrar sea siempre recuperable.

## No objetivos

- Deshacer un renombrado con Ctrl+Z (decidido: no; se corrige renombrando de nuevo).
- Mover archivos entre carpetas, duplicar o crear (ya existen «nuevo archivo/carpeta»).
- Copias `.bak` en estas operaciones.

## Antiobjetivos (sería un fallo aunque «funcione»)

- Perder contenido sin posibilidad de recuperarlo (borrado definitivo, reemplazo silencioso).
- Romper enlaces sin que el usuario lo supiera.
- Perder los cambios sin guardar del documento abierto.

## Comportamiento decidido por el usuario

### Cómo se lanza
- Clic derecho sobre un archivo o carpeta del panel Archivos: «Renombrar» y «Borrar».
- Atajos con el elemento seleccionado: **F2** renombra, **Supr** borra.

### Qué cuenta como enlace
- `href` y `src` en los HTML.
- `url(...)` y `@import` en los `.css`.
- Nada más (no se miran `srcset`, `<style>` en línea, JS, etc.).

### Borrar
- Siempre pide confirmación, con **un solo botón de confirmar** (sin escribir el nombre). El aviso dice cuántos archivos y subcarpetas se borran (en carpetas) y cuántos enlaces de otros HTML/CSS se rompen, listando los archivos afectados.
- Destino: **Papelera de reciclaje de Windows**. Sin copia `.bak`.
- Si el archivo (o una carpeta que lo contiene) está abierto en el editor: el documento **sigue abierto como documento sin ruta**, con su contenido y cambios sin guardar intactos.
- Los enlaces rotos solo se avisan; no hay nada que actualizar.
- Tras borrar el archivo abierto, **Guardar equivale a «Guardar como…»**: se elige dónde guardar y el archivo borrado no se recrea solo.

### Renombrar
- Si otros archivos enlazan al elemento (o a algo dentro de la carpeta), el aviso los lista y ofrece tres botones: **«Renombrar y actualizar los enlaces»**, **«Renombrar sin tocar los enlaces»** y **«Cancelar»**. Sin enlaces afectados, renombra directamente.
- Si el nuevo nombre ya existe en esa carpeta: **pregunta si reemplazar**; el existente va a la Papelera y el renombrado ocupa su lugar. Vale **cualquier combinación** de tipos (archivo por carpeta, carpeta por archivo, etc.), con el mismo aviso de reemplazo.
- Si cambia la **extensión** (incluido quitarla o añadirla): confirmación extra avisando de que cambia el tipo de archivo.
- Si el documento abierto cambia de ruta (porque se renombra él o una carpeta que lo contiene): **sigue abierto** con la ruta nueva.
- **No se puede deshacer** con Ctrl+Z.

## Restricciones

- Solo se opera dentro de la raíz del sitio (`resolve_in_root` ya impide salirse de ella).
- Reglas del proyecto: nunca automatizar ratón/teclado ni lanzar la GUI para verificar; se verifica con tests y Edge headless.
- Las ediciones de enlaces en HTML deben conservar el formato del archivo (la aplicación ya cuida esto en el guardado; ver `docs/fase2-conservar-formato.md`): cambiar solo el valor del atributo, no reformatear el resto.

## Suposiciones del autor (a revisar)

- Mi suposición: la lógica va en `crates/temiktli-shell/src/site.rs` (ya contiene `list_site`, `resolve_in_root`, `href_to_rel`, `is_linked`, `insert_link`), con tests primero; la interfaz en el panel Archivos y mensajes nuevos en `main.rs`.
- Mi suposición: se usa la API de Windows para enviar a la Papelera (por ejemplo `SHFileOperationW` con `FOF_ALLOWUNDO`, o la crate `trash`), no `std::fs::remove_*`.
- Mi suposición: los enlaces se resuelven contra la ubicación del archivo que los contiene (relativos), y también rutas absolutas del sitio (`/img/a.png`) respecto a la raíz; se ignoran URL externas, `mailto:`, `#ancla` y `data:`. Se conservan `?consulta` y `#ancla` al actualizar.
- Mi suposición: si el nombre nuevo es inválido en Windows (caracteres prohibidos, nombres reservados, vacío), se rechaza con un mensaje, sin diálogo de decisión.
- Mi suposición: al actualizar enlaces en archivos que están abiertos con cambios sin guardar, se actualizan en el documento abierto y no en disco, para no pisar el trabajo del usuario.

## Decisiones técnicas tomadas tras la revisión del constructor (mi criterio, no del usuario)

- Resolutor de enlaces nuevo, relativo al archivo que contiene el enlace y que admite `../` y rutas que empiezan por `/`; `href_to_rel` (site.rs:70) hoy rechaza ambos, así que se deja como está y se añade una función aparte. Escáner propio de atributos: comillas dobles, simples y sin comillas; tolera `%20`.
- Si hay varios buffers CSS abiertos (`css_files`, cada uno con su `rel`) o `active_file`, se actualizan todos con la misma regla al renombrar un archivo o una carpeta que los contiene.
- Papelera: preferencia por la crate `trash`. Si el envío a la Papelera falla, **nunca** se borra de forma definitiva: se avisa del error. La lógica se aísla tras una función para poder testear el resto sin Papelera real.
- Para contar y buscar enlaces se recorre el disco sin los límites de `list_site` (`MAX_DEPTH` 4, `MAX_ENTRIES` 800), omitiendo ocultos y enlaces simbólicos. Al borrar una carpeta se envían también sus `.bak` y ocultos; el conteo del aviso puede no coincidir con lo visible en el panel.
- Diálogos con `rfd::MessageDialog` (`YesNoCancelCustom` para los tres botones). El nombre nuevo se escribe en un campo en línea del panel Archivos, no en un diálogo. Validación de nombres con `check_segments`; comparación de existencia con `rel_key`, de modo que cambiar solo mayúsculas (`a.css` → `A.css`) no cuenta como «ya existe».
- Si el nombre no cambia, no se hace nada. F2 y Supr solo actúan con el foco en el panel Archivos.

## Para quien lo construya

- Orden sugerido: (1) lógica pura y testeada: búsqueda de referencias afectadas, cálculo de rutas nuevas, reescritura de enlaces conservando formato; (2) operaciones de disco (renombrar, Papelera, reemplazo); (3) interfaz: menú contextual, atajos, diálogos y estado del documento abierto.
- Después de implementar: `cargo test --workspace` sin warnings; verificar lo visual con Edge headless; indicar qué solo puede confirmar el usuario en su ventana.
- Pendiente aparte (no entra aquí): borrar `assets/sample.html` si el usuario lo confirma.
