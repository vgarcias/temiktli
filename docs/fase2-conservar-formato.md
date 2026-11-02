# Fase 2, tarea 1: conservar el formato original al guardar

Proyecto educativo e independiente. Sin afiliación con Adobe ni Macromedia.

## Problema
`Document::to_clean_html()` reescribe todo el archivo con el serializador HTML5
(comillas dobles, etiquetas en minúscula, cierres explícitos, `&copy;` → `©`,
espacios entre doctype/html/head perdidos…). Cambiar una línea ensucia todo el diff.
Hoy mitigado con `archivo.html.bak` (ver `backup_original` en `temiktli-shell`).

## Hallazgos del experimento (html5ever 0.39, tokenizador alimentado carácter a carácter)
Funciona:
- Rango exacto de etiquetas de inicio, incluso con `>` dentro de un atributo,
  mayúsculas, `<br/>`, y `<script>` que contiene `</div>`.
- `TreeSink::create_element` + token actual (nombre igual y `StartTag`) enlaza cada
  elemento con la etiqueta que lo creó. Los implícitos (html/head/body) quedan sin rango.

No funciona / trampas:
- `TreeSink::pop` NO se llama para la mayoría de cierres (`pop_until*` saca de
  `open_elems` sin avisar). `open_elems` es privado. No hay forma de leer cierres del parser.
- Desfase de 1 byte: texto pendiente al llegar `<` (`&x<b>` dio `b>`), y `\r` retenido
  (`\r\n<script>` dio `\n<script>`). Hay que VALIDAR cada rango de etiqueta
  (empieza por `<`, nombre coincide sin mayúsculas, termina en `>`) y corregir ±1;
  si no cuadra, ese elemento se marca sin rango.
- Los tokens de texto se emiten de forma diferida; no usar su rango como verdad.

## Diseño
1. **Lista de tokens** contigua con rangos validados (inicio, fin, comentario, doctype, texto).
2. **Elemento → etiqueta de inicio** por el hook de `create_element`.
3. **Simulación posterior** (sin ayuda del parser): recorrer tokens con una pila de
   abiertos y el DOM final. `EndTag N` cierra explícitamente el elemento abierto más alto
   llamado N; los de encima se cierran implícitamente al empezar el token que los saca
   (extensión termina donde empieza ese token). Esto da, por elemento:
   `start_tag`, `end_tag: Option`, `extent_end`.
4. **Instantánea original** por `NodeId`: atributos, texto (si solo texto), padre.
5. **Guardar = parches sobre el texto original**, derivados de comparar DOM actual vs
   instantánea (no de repetir comandos):
   - atributos distintos → reescribir SOLO esa etiqueta de inicio;
   - texto distinto (elemento solo-texto) → reemplazar el texto entre su etiqueta de
     inicio y el siguiente token de etiqueta;
   - elemento borrado → quitar su extensión;
   - elemento nuevo → insertar su serialización tras la extensión del hermano anterior,
     o tras la etiqueta de inicio del padre si es el primero.
   Todo lo demás se copia byte a byte. Los ancestros borrados/insertados absorben a
   sus descendientes.
6. **Verificación**: re-parsear el resultado; su serialización normalizada debe ser igual a
   la del DOM actual. Si no coincide (o falta algún rango) → guardado normalizado de hoy,
   con aviso en la barra de estado. Nunca peor que el comportamiento actual.

## Invariantes a probar
- Sin ediciones: salida == entrada, byte a byte, en HTML bien formado.
- Una edición de atributo/texto/inserción/borrado cambia SOLO esa zona.
- Casos difíciles: `>` en atributo, CRLF, `<p>`/`<li>` sin cerrar, entidades, `<script>`,
  comentarios, HTML sin html/head/body, UTF-8 multibyte.
- Cualquier entrada: nunca pánico, y el resultado siempre pasa la verificación o cae al
  guardado normalizado.

## Fuera de alcance (por ahora)
Mover nodos (no existe comando), `<template>`, fragmentos de plantillas (`<?php`),
adopción de formateo (`<b><i></b></i>`): caen al guardado normalizado vía verificación.

## Estado
- [x] Experimento de rangos (scratchpad, descartable).
- [x] `spanparse.rs`: tokenizador con rangos validados (`scan_tag`, `locate`).
- [x] `source.rs`: simulación de pila → extensiones (`SourceMap::build`).
- [x] Instantánea y derivación de parches (`SourceMap::render`: atributos, texto, borrado, inserción).
- [x] Verificación por huella canónica y reserva al guardado normalizado (`Document::to_saved_html`).
- [x] Integración en `App::save` + mensaje de estado + `rebase_source` tras guardar.
- [ ] Prueba manual en la GUI (guardar un archivo real con formato propio y comparar con el `.bak`).

## Limitaciones conocidas de la versión actual
- Una inserción se escribe sin sangría ni saltos de línea propios (válido, pero poco bonito).
- Mover nodos, `<template>`, mala anidación con formato y tablas con *foster parenting*
  caen al guardado normalizado (se avisa en la barra de estado).
- Un atributo duplicado en el original, o con mayúsculas SVG (`viewBox`), cae al normalizado.
- Si el nuevo texto de un `<textarea>`/`<pre>` empieza por salto de línea se añade uno extra.
