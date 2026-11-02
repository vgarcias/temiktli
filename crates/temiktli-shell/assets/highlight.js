// Coloreado de sintaxis para la vista de código: HTML, con CSS dentro de <style> y
// JavaScript dentro de <script>. Convierte texto en HTML escapado con <span class="c-…">.
// Invariante: el texto visible del resultado es exactamente el texto de entrada.
'use strict';
(function (root) {
  const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

  // Acumula tramos contiguos de la misma clase para no generar un <span> por carácter.
  function makeSink() {
    let out = '', cls = null, buf = '';
    const flush = () => {
      if (!buf) return;
      out += cls ? '<span class="' + cls + '">' + esc(buf) + '</span>' : esc(buf);
      buf = '';
    };
    return {
      put(c, text) {
        if (!text) return;
        if (c !== cls) { flush(); cls = c; }
        buf += text;
      },
      done() { flush(); return out; },
    };
  }

  // Búsqueda "pegajosa": coincidencia de `re` exactamente en la posición `i`.
  function at(re, src, i) {
    re.lastIndex = i;
    return re.exec(src);
  }

  const RE = {
    ws: /\s+/y,
    tagName: /[^\s\/>]+/y,
    attrName: /[^\s\/>=]+/y,
    unquoted: /[^\s>]+/y,
    entity: /&(#x[0-9a-f]+|#\d+|\w+);/iy,
    ident: /[-\w]+/y,
    num: /-?(\d+\.?\d*|\.\d+)(%|[a-z]+)?/iy,
    hex: /#[0-9a-f]{3,8}\b/iy,
    fn: /[-\w]+(?=\()/y,
    at: /@[-\w]+/y,
    jsNum: /0x[0-9a-f_]+|\d[\d_.]*(e[+-]?\d+)?n?/iy,
    jsWord: /[A-Za-z_$][\w$]*/y,
  };

  function quoted(src, i) {
    const q = src[i];
    let j = i + 1;
    while (j < src.length && src[j] !== q) j += src[j] === '\\' ? 2 : 1;
    return Math.min(j + 1, src.length);
  }

  // ---------- CSS ----------
  const CSS_NESTING = /^@(media|supports|layer|container)\b/i;

  function css(src, sink) {
    const n = src.length;
    const stack = []; // 'rules' (selectores) o 'decls' (propiedades)
    let prelude = 0, inValue = false, i = 0;
    const mode = () => stack[stack.length - 1] || 'rules';
    while (i < n) {
      const c = src[i];
      if (c === '/' && src[i + 1] === '*') {
        const e = src.indexOf('*/', i + 2);
        const end = e < 0 ? n : e + 2;
        sink.put('c-comment', src.slice(i, end)); i = end; continue;
      }
      if (c === '"' || c === "'") {
        const end = quoted(src, i);
        sink.put('c-str', src.slice(i, end)); i = end; continue;
      }
      if (c === '{') {
        const head = src.slice(prelude, i).trim();
        stack.push(CSS_NESTING.test(head) ? 'rules' : 'decls');
        sink.put('c-punct', c); i++; prelude = i; inValue = false; continue;
      }
      if (c === '}') {
        stack.pop(); inValue = false;
        sink.put('c-punct', c); i++; prelude = i; continue;
      }
      if (c === ';') {
        inValue = false; sink.put('c-punct', c); i++;
        if (mode() === 'rules') prelude = i;
        continue;
      }
      let m;
      if (mode() === 'decls') {
        if (!inValue) {
          if (c === ':') { inValue = true; sink.put('c-punct', c); i++; continue; }
          if ((m = at(RE.ident, src, i))) { sink.put('c-prop', m[0]); i += m[0].length; continue; }
        } else {
          if ((m = at(RE.hex, src, i)) || (m = at(RE.num, src, i))) { sink.put('c-num', m[0]); i += m[0].length; continue; }
          if (c === '!') { sink.put('c-at', '!'); i++; continue; }
          if ((m = at(RE.fn, src, i))) { sink.put('c-fn', m[0]); i += m[0].length; continue; }
          if ((m = at(RE.ident, src, i))) { sink.put('', m[0]); i += m[0].length; continue; }
        }
        sink.put('', c); i++; continue;
      }
      // Selectores y cabeceras de at-rules
      if ((m = at(RE.at, src, i))) { sink.put('c-at', m[0]); i += m[0].length; continue; }
      const isAtPrelude = src.slice(prelude, i).trimStart().startsWith('@');
      let j = i + 1;
      while (j < n && !'{};"\'/@'.includes(src[j])) j++;
      sink.put(isAtPrelude ? '' : 'c-sel', src.slice(i, j)); i = j;
    }
  }

  // ---------- JavaScript (sencillo: comentarios, cadenas, números y palabras clave) ----------
  const JS_KEYWORDS = new Set(('const let var function return if else for while do switch case break ' +
    'continue new this class extends import export from default try catch finally throw async await ' +
    'typeof instanceof in of null true false undefined void delete yield static super').split(' '));

  function js(src, sink) {
    const n = src.length;
    let i = 0, m;
    while (i < n) {
      const c = src[i];
      if (c === '/' && src[i + 1] === '/') {
        let e = src.indexOf('\n', i); if (e < 0) e = n;
        sink.put('c-comment', src.slice(i, e)); i = e; continue;
      }
      if (c === '/' && src[i + 1] === '*') {
        const e = src.indexOf('*/', i + 2); const end = e < 0 ? n : e + 2;
        sink.put('c-comment', src.slice(i, end)); i = end; continue;
      }
      if (c === '"' || c === "'" || c === '`') {
        const end = quoted(src, i);
        sink.put('c-str', src.slice(i, end)); i = end; continue;
      }
      if ((m = at(RE.jsNum, src, i))) { sink.put('c-num', m[0]); i += m[0].length; continue; }
      if ((m = at(RE.jsWord, src, i))) {
        sink.put(JS_KEYWORDS.has(m[0]) ? 'c-kw' : '', m[0]); i += m[0].length; continue;
      }
      sink.put('', c); i++;
    }
  }

  // ---------- HTML ----------
  function html(src, sink) {
    const n = src.length;
    let lower = null, i = 0, m;
    while (i < n) {
      const c = src[i];
      if (src.startsWith('<!--', i)) {
        const e = src.indexOf('-->', i + 4); const end = e < 0 ? n : e + 3;
        sink.put('c-comment', src.slice(i, end)); i = end; continue;
      }
      if (c === '<' && (src[i + 1] === '!' || src[i + 1] === '?')) {
        const e = src.indexOf('>', i); const end = e < 0 ? n : e + 1;
        sink.put('c-doctype', src.slice(i, end)); i = end; continue;
      }
      if (c === '<' && /[A-Za-z\/]/.test(src[i + 1] || '')) {
        let j = i + 1;
        const closing = src[j] === '/';
        if (closing) j++;
        if (!(m = at(RE.tagName, src, j))) { sink.put('', c); i++; continue; }
        sink.put('c-punct', closing ? '</' : '<');
        sink.put('c-tag', m[0]);
        j += m[0].length;
        const tag = m[0].toLowerCase();
        let selfClosed = false;
        while (j < n) {
          if ((m = at(RE.ws, src, j))) { sink.put('', m[0]); j += m[0].length; continue; }
          const ch = src[j];
          if (ch === '>') { sink.put('c-punct', '>'); j++; break; }
          if (ch === '/') {
            if (src[j + 1] === '>') { sink.put('c-punct', '/>'); j += 2; selfClosed = true; break; }
            sink.put('c-punct', '/'); j++; continue;
          }
          if (ch === '=') { sink.put('c-punct', '='); j++; continue; }
          if (!(m = at(RE.attrName, src, j))) { sink.put('', ch); j++; continue; }
          sink.put('c-attr', m[0]); j += m[0].length;
          let k = j;
          while (k < n && /\s/.test(src[k])) k++;
          if (src[k] !== '=') continue;
          sink.put('', src.slice(j, k)); sink.put('c-punct', '='); k++;
          const ws0 = k;
          while (k < n && /\s/.test(src[k])) k++;
          sink.put('', src.slice(ws0, k));
          if (src[k] === '"' || src[k] === "'") {
            const end = quoted(src, k);
            sink.put('c-str', src.slice(k, end)); j = end;
          } else if ((m = at(RE.unquoted, src, k))) {
            sink.put('c-str', m[0]); j = k + m[0].length;
          } else {
            j = k;
          }
        }
        i = j;
        if (!closing && !selfClosed && (tag === 'style' || tag === 'script')) {
          if (lower === null) lower = src.toLowerCase();
          const idx = lower.indexOf('</' + tag, i);
          const end = idx < 0 ? n : idx;
          const body = src.slice(i, end);
          if (tag === 'style') css(body, sink); else js(body, sink);
          i = end;
        }
        continue;
      }
      if (c === '&') {
        if ((m = at(RE.entity, src, i))) { sink.put('c-ent', m[0]); i += m[0].length; continue; }
        sink.put('', c); i++; continue;
      }
      let j = i + 1;
      while (j < n && src[j] !== '<' && src[j] !== '&') j++;
      sink.put('', src.slice(i, j)); i = j;
    }
  }

  function highlight(src) {
    const sink = makeSink();
    html(src, sink);
    return sink.done();
  }

  // Hoja de estilo suelta (un archivo .css).
  function highlightCss(src) {
    const sink = makeSink();
    css(src, sink);
    return sink.done();
  }

  root.highlightHtml = highlight;
  root.highlightCss = highlightCss;
  if (typeof module !== 'undefined') module.exports = { highlight, highlightCss };
})(typeof window !== 'undefined' ? window : globalThis);
