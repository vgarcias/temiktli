// Autocompletado de la vista de código: sabe dónde está el cursor (nombre de etiqueta,
// atributo, valor, propiedad CSS…) y qué sugerir. Es lógica pura sobre (texto, posición);
// la interfaz (lista, teclas) está en code.html.
'use strict';
(function (root) {
  const VOID = new Set('area base br col embed hr img input link meta source track wbr'.split(' '));

  const TAGS = ('a abbr address article aside audio b blockquote body br button canvas caption cite ' +
    'code col colgroup dd details div dl dt em fieldset figcaption figure footer form h1 h2 h3 h4 h5 ' +
    'h6 head header hr html i iframe img input label legend li link main mark meta nav noscript ol ' +
    'optgroup option p picture pre script section select small source span strong style sub summary ' +
    'sup svg table tbody td template textarea tfoot th thead time title tr ul video').split(' ');

  const GLOBAL_ATTRS = ('class id style title lang dir hidden tabindex role contenteditable draggable ' +
    'translate aria-label aria-hidden aria-describedby data-').split(' ');

  const ATTRS_BY_TAG = {
    a: 'href target rel download hreflang type',
    img: 'src alt width height loading srcset sizes decoding',
    input: 'type name value placeholder required disabled checked min max step pattern readonly autocomplete maxlength',
    textarea: 'name rows cols placeholder required disabled readonly maxlength',
    select: 'name multiple required disabled',
    option: 'value selected disabled',
    button: 'type name value disabled',
    form: 'action method enctype novalidate target',
    label: 'for',
    link: 'rel href type media sizes',
    meta: 'charset name content property http-equiv',
    script: 'src type async defer crossorigin integrity',
    style: 'media type',
    iframe: 'src title width height loading allow sandbox',
    video: 'src poster controls autoplay loop muted preload width height',
    audio: 'src controls autoplay loop muted preload',
    source: 'src srcset type media sizes',
    td: 'colspan rowspan headers',
    th: 'colspan rowspan scope headers',
    html: 'lang',
    ol: 'start reversed type',
    time: 'datetime',
    details: 'open',
  };
  for (const k of Object.keys(ATTRS_BY_TAG)) ATTRS_BY_TAG[k] = ATTRS_BY_TAG[k].split(' ');

  // Atributos que se escriben sin valor.
  const BOOLEAN_ATTRS = new Set(('hidden disabled checked required readonly selected multiple async defer ' +
    'controls autoplay loop muted novalidate open reversed').split(' '));

  const CSS_PROPS = ('align-items align-self animation background background-color background-image ' +
    'background-position background-repeat background-size border border-bottom border-collapse ' +
    'border-color border-left border-radius border-right border-style border-top border-width bottom ' +
    'box-shadow box-sizing clear color column-gap content cursor display filter flex flex-basis ' +
    'flex-direction flex-grow flex-shrink flex-wrap float font font-family font-size font-style ' +
    'font-weight gap grid grid-area grid-template-areas grid-template-columns grid-template-rows ' +
    'height inset justify-content justify-items left letter-spacing line-height list-style margin ' +
    'margin-bottom margin-left margin-right margin-top max-height max-width min-height min-width ' +
    'object-fit opacity order outline overflow overflow-x overflow-y padding padding-bottom ' +
    'padding-left padding-right padding-top place-items pointer-events position right row-gap ' +
    'text-align text-decoration text-overflow text-transform top transform transition ' +
    'vertical-align visibility white-space width word-break z-index').split(' ');

  const CSS_AT = {
    '@media': '@media (max-width: 600px) {\n  \n}',
    '@supports': '@supports (display: grid) {\n  \n}',
    '@font-face': '@font-face {\n  \n}',
    '@keyframes': '@keyframes nombre {\n  from {  }\n  to {  }\n}',
    '@layer': '@layer base {\n  \n}',
    '@container': '@container (min-width: 400px) {\n  \n}',
    '@import': '@import url("");',
  };
  // Caracteres entre el final de la plantilla y el lugar donde queda el cursor.
  const CSS_AT_BACK = { '@media': 2, '@supports': 2, '@font-face': 2, '@layer': 2, '@container': 2, '@keyframes': 25, '@import': 3 };

  const RAW_IN_STYLE = /^@(media|supports|layer|container)\b/i;

  // ---------- Contexto del cursor ----------

  // Estado del CSS entre `from` y `n` (cursor): ¿propiedad, valor, selector, at-rule?
  function cssContext(text, from, n, res) {
    const blocks = [];
    let inComment = false, quote = null, segStart = from, preludeStart = from;
    for (let p = from; p < n; p++) {
      const ch = text[p];
      if (inComment) { if (ch === '*' && text[p + 1] === '/') { inComment = false; p++; } continue; }
      if (quote) { if (ch === '\\') p++; else if (ch === quote) quote = null; continue; }
      if (ch === '/' && text[p + 1] === '*') { inComment = true; p++; continue; }
      if (ch === '"' || ch === "'") { quote = ch; continue; }
      if (ch === '{') {
        blocks.push(RAW_IN_STYLE.test(text.slice(preludeStart, p).trim()) ? 'rules' : 'decls');
        segStart = preludeStart = p + 1;
      } else if (ch === '}') {
        blocks.pop(); segStart = preludeStart = p + 1;
      } else if (ch === ';') {
        segStart = p + 1;
        if ((blocks[blocks.length - 1] || 'rules') === 'rules') preludeStart = p + 1;
      }
    }
    if (inComment || quote) return res('none');
    const seg = text.slice(segStart, n);
    if ((blocks[blocks.length - 1] || 'rules') === 'decls') {
      if (seg.includes(':')) return res('none'); // posición de valor
      const prefix = /[-\w]*$/.exec(seg)[0];
      return res('css-prop', { prefix, from: n - prefix.length });
    }
    const at = /@[-\w]*$/.exec(seg);
    return at ? res('css-at', { prefix: at[0], from: n - at[0].length }) : res('none');
  }

  // Devuelve { kind, prefix, from, tag, stack, ended, quote }.
  //  kind: none | tag | close | attr | attrvalue | quote | css-prop | css-at
  //  from: dónde empieza lo que se está escribiendo (para reemplazarlo al aceptar)
  //  stack: elementos abiertos antes del cursor (para cerrar el más interno)
  //  ended: si el carácter justo antes del cursor cierra una etiqueta de inicio
  // Con `{ css: true }` el texto es una hoja de estilo suelta, no un documento HTML.
  function analyze(text, pos, opts) {
    const n = Math.min(pos, text.length);
    const stack = [];
    let ended = null;
    const res = (kind, extra) =>
      Object.assign({ kind, prefix: '', from: n, tag: '', stack: stack.slice(), ended, quote: null }, extra);
    if (opts && opts.css) return cssContext(text, 0, n, res);
    let lower = null;
    let i = 0;
    while (i < n) {
      if (text.startsWith('<!--', i)) {
        const e = text.indexOf('-->', i + 4);
        if (e < 0 || e + 3 > n) return res('none');
        i = e + 3; continue;
      }
      if (text[i] !== '<') { i++; continue; }
      const nx = text[i + 1];
      if (nx === '!' || nx === '?') {
        const e = text.indexOf('>', i);
        if (e < 0 || e + 1 > n) return res('none');
        i = e + 1; continue;
      }
      if (!(nx && /[A-Za-z\/]/.test(nx))) {
        if (i + 1 >= n) return res('tag', { from: n });
        i++; continue;
      }
      const closing = nx === '/';
      const j = i + (closing ? 2 : 1);
      let k = j;
      while (k < text.length && !/[\s\/>]/.test(text[k])) k++;
      if (k >= n) {
        // Con el cursor justo tras «<» y antes de «/», `j` queda a la derecha del cursor.
        const prefix = text.slice(j, n);
        return res(closing ? 'close' : 'tag', { from: Math.min(j, n), prefix });
      }
      const name = text.slice(j, k).toLowerCase();
      let q = null, m = k;
      for (; m < n; m++) {
        const ch = text[m];
        if (q) { if (ch === q) q = null; continue; }
        if (ch === '"' || ch === "'") { q = ch; continue; }
        if (ch === '>') break;
      }
      if (m >= n) { // el cursor está dentro de la etiqueta
        if (q) return res('quote', { quote: q, tag: name });
        if (closing) return res('none');
        let s = n;
        while (s > k && /[\w:-]/.test(text[s - 1])) s--;
        const before = text[s - 1];
        if (before === '=') return res('attrvalue', { tag: name });
        if (s > k && !/[\s"']/.test(before)) return res('none');
        return res('attr', { from: s, prefix: text.slice(s, n), tag: name });
      }
      const selfClosing = text[m - 1] === '/';
      if (closing) {
        const idx = stack.lastIndexOf(name);
        if (idx >= 0) stack.length = idx;
      } else {
        if (m === n - 1) ended = { name, void: VOID.has(name), selfClosing };
        if (!VOID.has(name) && !selfClosing) stack.push(name);
        if ((name === 'style' || name === 'script') && !selfClosing) {
          if (lower === null) lower = text.toLowerCase();
          const e = lower.indexOf('</' + name, m + 1);
          if (e < 0 || e >= n) return name === 'style' ? cssContext(text, m + 1, n, res) : res('none');
          i = e; continue;
        }
      }
      i = m + 1;
    }
    return res('none');
  }

  // ---------- Sugerencias ----------

  const MAX_ITEMS = 12;

  // Devuelve [{ label, insert, back }]: `insert` reemplaza lo escrito desde `from`;
  // `back` = caracteres hacia atrás desde el final de lo insertado donde queda el cursor.
  // Sin `force`, atributos y CSS esperan a que haya al menos una letra escrita.
  function suggest(a, force) {
    const p = a.prefix.toLowerCase();
    const starts = (list) => list.filter((x) => x.toLowerCase().startsWith(p) && x.toLowerCase() !== p);
    let out = [];
    switch (a.kind) {
      case 'tag':
        out = starts(TAGS).map((t) => ({ label: t, insert: t, back: 0 }));
        break;
      case 'close': {
        const top = a.stack[a.stack.length - 1];
        if (top && top.startsWith(p)) out = [{ label: '</' + top + '>', insert: top + '>', back: 0 }];
        break;
      }
      case 'attr': {
        if (!p && !force) break;
        const names = [...(ATTRS_BY_TAG[a.tag] || []), ...GLOBAL_ATTRS];
        out = starts([...new Set(names)]).map((n) =>
          n.endsWith('-') || BOOLEAN_ATTRS.has(n)
            ? { label: n, insert: n, back: 0 }
            : { label: n, insert: n + '=""', back: 1 });
        break;
      }
      case 'css-prop':
        if (!p && !force) break;
        out = starts(CSS_PROPS).map((n) => ({ label: n, insert: n + ': ;', back: 1 }));
        break;
      case 'css-at':
        out = starts(Object.keys(CSS_AT)).map((n) => ({ label: n, insert: CSS_AT[n], back: CSS_AT_BACK[n] }));
        break;
      default:
        break;
    }
    return out.slice(0, MAX_ITEMS);
  }

  // ¿Hay que insertar el cierre tras escribir '>'? Devuelve el texto, o null.
  function closingFor(text, pos) {
    const a = analyze(text, pos);
    if (!a.ended || a.ended.void || a.ended.selfClosing) return null;
    const close = '</' + a.ended.name + '>';
    return text.slice(pos).toLowerCase().startsWith(close) ? null : close;
  }

  root.htmlComplete = { analyze, suggest, closingFor, VOID };
  if (typeof module !== 'undefined') module.exports = root.htmlComplete;
})(typeof window !== 'undefined' ? window : globalThis);
