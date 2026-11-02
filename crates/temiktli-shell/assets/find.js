// Buscar y reemplazar para la vista de código. Lógica pura sobre (texto, consulta,
// opciones); la barra de búsqueda y el cuadro de texto están en code.html.
'use strict';
(function (root) {
  const MAX_MATCHES = 5000;

  const escapeRegExp = (s) => s.replace(/[.*+?^${}()|[\]\\\/-]/g, '\\$&');
  const escapeHtml = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

  // opts: { caseSensitive, regex, word }. Devuelve { re } o { re: null, error }.
  function compile(query, opts) {
    const o = opts || {};
    if (!query) return { re: null, error: null };
    let src = o.regex ? query : escapeRegExp(query);
    // «Palabra completa» con letras Unicode (\b solo entiende ASCII).
    if (o.word) src = '(?<![\\p{L}\\p{N}_])(?:' + src + ')(?![\\p{L}\\p{N}_])';
    const flags = 'gm' + (o.caseSensitive ? '' : 'i') + (o.word ? 'u' : '');
    try {
      return { re: new RegExp(src, flags), error: null };
    } catch (e) {
      return { re: null, error: String(e.message || e).replace(/^Invalid regular expression: /, '') };
    }
  }

  // Coincidencias no vacías, en orden. `truncated` avisa de que hay más del límite.
  function findAll(text, re, limit) {
    const max = limit || MAX_MATCHES;
    const matches = [];
    if (!re) return { matches, truncated: false };
    re.lastIndex = 0;
    let m;
    while ((m = re.exec(text))) {
      if (m[0].length === 0) { re.lastIndex++; continue; } // p. ej. /a*/ coincide con «nada»
      if (matches.length >= max) return { matches, truncated: true };
      matches.push({ start: m.index, end: m.index + m[0].length });
    }
    return { matches, truncated: false };
  }

  // Cuál es el texto que sustituye a la coincidencia que empieza en `start`. En modo regex
  // valen $1, $&, $<nombre>…; en modo normal el texto es literal (un «$1» es «$1»).
  function replacementAt(text, start, re, replacement, regexMode) {
    const sticky = new RegExp(re.source, re.flags.replace('g', '') + 'y');
    sticky.lastIndex = start;
    const m = sticky.exec(text);
    if (!m) return null;
    const end = start + m[0].length;
    sticky.lastIndex = start;
    const replaced = text.replace(sticky, regexMode ? replacement : () => replacement);
    return replaced.slice(start, replaced.length - (text.length - end));
  }

  // Sustituye todas las coincidencias (las vacías no cuentan). Devuelve { text, count }.
  function replaceAll(text, re, replacement, regexMode) {
    const { matches } = findAll(text, re, Infinity);
    let out = '', last = 0;
    for (const m of matches) {
      out += text.slice(last, m.start) + replacementAt(text, m.start, re, replacement, regexMode);
      last = m.end;
    }
    return { text: out + text.slice(last), count: matches.length };
  }

  // Índice de la coincidencia que sigue (dir=1) o precede (dir=-1) a `pos`, dando la vuelta.
  function stepFrom(matches, pos, dir) {
    if (!matches.length) return -1;
    if (dir >= 0) {
      const i = matches.findIndex((m) => m.start >= pos);
      return i < 0 ? 0 : i;
    }
    for (let i = matches.length - 1; i >= 0; i--) if (matches[i].end <= pos) return i;
    return matches.length - 1;
  }

  // Texto escapado con <mark> en cada coincidencia; `cur` es la actual. El texto visible es
  // exactamente el original (la capa se dibuja detrás del cuadro de texto).
  function marksHtml(text, matches, cur) {
    let out = '', last = 0;
    matches.forEach((m, i) => {
      out += escapeHtml(text.slice(last, m.start));
      out += '<mark class="' + (i === cur ? 'cur' : 'm') + '">' + escapeHtml(text.slice(m.start, m.end)) + '</mark>';
      last = m.end;
    });
    return out + escapeHtml(text.slice(last));
  }

  // Línea (0-based) y columna (contando tabuladores a 2) de una posición.
  function lineCol(text, pos) {
    const before = text.slice(0, pos);
    const line = before.split('\n').length - 1;
    let col = 0;
    for (const ch of before.slice(before.lastIndexOf('\n') + 1)) col += ch === '\t' ? 2 - (col % 2) : 1;
    return { line, col };
  }

  root.htmlFind = { compile, findAll, replacementAt, replaceAll, stepFrom, marksHtml, lineCol, MAX_MATCHES };
  if (typeof module !== 'undefined') module.exports = root.htmlFind;
})(typeof window !== 'undefined' ? window : globalThis);
