// Se inyecta en cada carga del lienzo. Rust es la fuente de verdad: este script
// solo (1) traduce la interacción en mensajes y (2) aplica los Patches que llegan.
(function () {
  // Textos de la interfaz: Rust sustituye el marcador por el diccionario plano del idioma activo
  // (clave con puntos → texto). Si falta una clave se ve entre corchetes, nunca en silencio.
  const I18N = /*I18N_JSON*/{};
  const t = (key, args) => {
    const text = I18N[key];
    if (typeof text !== 'string') return '[' + key + ']';
    return args ? text.replace(/\{(\w+)\}/g, (m, name) => (name in args ? String(args[name]) : m)) : text;
  };

  const VOID = new Set(['area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input',
                        'link', 'meta', 'source', 'track', 'wbr']);
  let overlay = null;
  let label = null;
  let selected = null;
  let editing = null; // { el, original }

  const send = (msg) => window.ipc.postMessage(JSON.stringify(msg));
  const byId = (id) => document.querySelector('[data-tk-id="' + id + '"]');
  const idOf = (el) => Number(el.getAttribute('data-tk-id'));

  // ---------- Fondo ----------
  // Un navegador pinta el lienzo de blanco si ni <html> ni <body> definen fondo. El
  // webview es transparente y dejaría ver el fondo de la ventana (oscuro en Windows
  // con tema oscuro). Solo se aplica si la página no tiene fondo propio, para no
  // romper la propagación del fondo de <body> al lienzo.
  function ensureCanvasBackground() {
    const clear = (c) => !c || c === 'transparent' || c === 'rgba(0, 0, 0, 0)';
    const root = document.documentElement;
    root.style.removeProperty('background-color');
    const html = getComputedStyle(root).backgroundColor;
    const body = document.body ? getComputedStyle(document.body).backgroundColor : '';
    if (clear(html) && clear(body)) root.style.setProperty('background-color', '#fff');
  }

  // ---------- Scroll entre recargas ----------
  // Editar el código recarga el lienzo entero; el scroll se guarda al salir y se repone
  // al volver a cargar. Si la URL trae `fresh=1` (otro documento) no se repone.
  const SCROLL_KEY = 'tk-scroll';
  const fresh = /[?&]fresh=1\b/.test(location.search);
  const storage = () => { try { return window.sessionStorage; } catch (_) { return null; } };
  window.addEventListener('pagehide', () => {
    const s = storage();
    if (s) s.setItem(SCROLL_KEY, JSON.stringify([window.scrollX, window.scrollY]));
  });
  function restoreScroll() {
    const s = storage();
    if (fresh) { if (s) s.removeItem(SCROLL_KEY); return; }
    let pos = null;
    try { pos = JSON.parse((s && s.getItem(SCROLL_KEY)) || 'null'); } catch (_) { /* dato roto */ }
    if (Array.isArray(pos)) window.scrollTo(pos[0] || 0, pos[1] || 0);
  }
  document.addEventListener('DOMContentLoaded', restoreScroll);
  window.addEventListener('load', () => { restoreScroll(); const s = storage(); if (s) s.removeItem(SCROLL_KEY); });

  // ---------- Selección visual ----------
  function ensureOverlay() {
    if (overlay && overlay.isConnected) return overlay;
    overlay = document.createElement('div');
    overlay.setAttribute('data-tk-overlay', '');
    overlay.style.cssText =
      'position:absolute;pointer-events:none;z-index:2147483647;display:none;' +
      'outline:2px solid #3b82f6;background:rgba(59,130,246,.12);';
    label = document.createElement('span');
    // La pestaña azul es el asa para arrastrar el elemento (el recuadro sigue sin recibir clics).
    label.style.cssText =
      'position:absolute;left:-2px;bottom:100%;margin-bottom:2px;padding:1px 6px;' +
      'font:11px/16px system-ui,sans-serif;color:#fff;background:#3b82f6;white-space:nowrap;' +
      'pointer-events:auto;cursor:grab;user-select:none;-webkit-user-select:none;';
    label.title = t('canvas.drag_handle_title');
    label.addEventListener('mousedown', startDrag, true);
    overlay.appendChild(label);
    document.documentElement.appendChild(overlay);
    return overlay;
  }

  function describe(el) {
    let s = el.tagName.toLowerCase();
    if (el.id) s += '#' + el.id;
    const c = (el.getAttribute('class') || '').trim().split(/\s+/)[0];
    if (c) s += '.' + c;
    return s;
  }

  // El elemento que se dibuja: el seleccionado, o, si no tiene caja (`<script>`, `<style>`,
  // `display:none`…), su ancestro más cercano que sí la tenga.
  function visibleBox(el) {
    for (let e = el; e && e !== document.documentElement; e = e.parentElement) {
      const r = e.getBoundingClientRect();
      if (r.width > 0 && r.height > 0) return { el: e, rect: r };
    }
    return null;
  }

  // ---------- Cursor de inserción ----------
  // Un cursor parpadeante que marca dónde caerá el próximo elemento. Rust calcula el sitio
  // (padre + índice entre sus hijos, igual que `Insert`); aquí solo se coloca en pantalla.
  let mark = { parent: null, index: 0 };
  let markEl = null;

  function ensureMark() {
    if (markEl && markEl.isConnected) return markEl;
    markEl = document.createElement('div');
    markEl.setAttribute('data-tk-overlay', '');
    markEl.setAttribute('data-tk-insertmark', '');
    // Barra vertical como el cursor de un procesador de textos. Blanca con `difference`:
    // se ve negra sobre fondo claro y blanca sobre fondo oscuro.
    markEl.style.cssText =
      'position:absolute;pointer-events:none;z-index:2147483646;display:none;width:1px;' +
      'background:#fff;mix-blend-mode:difference;';
    try {
      markEl.animate(
        [{ opacity: 1 }, { opacity: 1, offset: 0.5 }, { opacity: 0, offset: 0.5 }, { opacity: 0 }],
        { duration: 1100, iterations: Infinity });
    } catch (_) { /* sin animaciones: queda fijo */ }
    document.documentElement.appendChild(markEl);
    return markEl;
  }

  // Primer elemento con caja yendo desde `from` en la dirección `step` entre los hijos de `parent`.
  function boxedSibling(parent, from, step) {
    for (let i = from; i >= 0 && i < parent.childNodes.length; i += step) {
      const n = parent.childNodes[i];
      if (n.nodeType !== 1 || n.hasAttribute('data-tk-overlay')) continue;
      const r = n.getBoundingClientRect();
      if (r.width > 0 && r.height > 0) return r;
    }
    return null;
  }

  function placeMark() {
    const m = ensureMark();
    const parent = mark.parent == null ? null : byId(mark.parent);
    // Mientras se escribe dentro de un elemento no se va a insertar otro: el cursor de inserción
    // se oculta hasta que acabe la edición (`commitEdit` / `cancelEdit` vuelven a colocarlo).
    if (editing || !parent) { m.style.display = 'none'; return; }
    const sx = window.scrollX, sy = window.scrollY;
    // El cursor va donde empezaría la línea nueva: al borde izquierdo, bajo el hermano
    // anterior (o sobre el siguiente, o al inicio del contenido del padre si está vacío).
    let left, top;
    const prev = boxedSibling(parent, mark.index - 1, -1);
    const next = prev ? null : boxedSibling(parent, mark.index, 1);
    if (prev) { left = prev.left; top = prev.bottom; }
    else if (next) { left = next.left; top = next.top; }
    else {
      const r = parent.getBoundingClientRect();
      const cs = getComputedStyle(parent);
      left = r.left + (parseFloat(cs.borderLeftWidth) || 0) + (parseFloat(cs.paddingLeft) || 0);
      top = r.top + (parseFloat(cs.borderTopWidth) || 0) + (parseFloat(cs.paddingTop) || 0);
    }
    // Altura de una línea de texto del padre.
    const cs = getComputedStyle(parent);
    const size = parseFloat(cs.fontSize) || 16;
    const line = parseFloat(cs.lineHeight) || size * 1.25; // «normal» no es un número
    // Fino (1 px) y de la altura de las letras, no de toda la línea, centrado en ella. Las
    // posiciones se redondean: una barra de 1 px en media posición se vería borrosa y gris.
    const lineBox = Math.max(Math.min(line, size * 2), 12);
    const height = Math.max(Math.round(size * 1.05), 10);
    m.style.display = 'block';
    m.style.left = Math.round(left + sx) + 'px';
    m.style.top = Math.round(top + sy + (lineBox - height) / 2) + 'px';
    m.style.height = height + 'px';
  }

  function highlight() {
    placeMark();
    const o = ensureOverlay();
    const el = selected == null ? null : byId(selected);
    const box = el && visibleBox(el);
    if (!box) { o.style.display = 'none'; return; }
    const r = box.rect;
    o.style.display = 'block';
    o.style.left = r.left + window.scrollX + 'px';
    o.style.top = r.top + window.scrollY + 'px';
    o.style.width = r.width + 'px';
    o.style.height = r.height + 'px';
    label.textContent = box.el === el ? describe(el) : t('canvas.no_box', { element: describe(el), container: describe(box.el) });
  }

  // ---------- Aplicar Patches de Rust ----------
  const patches = {
    Highlight(p) { selected = p.id; },
    InsertMark(p) { mark = { parent: p.parent, index: p.index }; },
    DropState(p) { dropOk = p.ok; dropWhy = p.why || ''; setDragCursor(); },
    SetAttr(p) {
      const el = byId(p.id);
      if (!el) return;
      if (p.value === null) el.removeAttribute(p.name); else el.setAttribute(p.name, p.value);
    },
    SetText(p) {
      const el = byId(p.id);
      if (el) el.textContent = p.text;
    },
    Insert(p) {
      const parent = byId(p.parent);
      if (!parent) return;
      const tpl = document.createElement('template');
      tpl.innerHTML = p.html;
      const node = tpl.content.firstChild;
      if (node) parent.insertBefore(node, parent.childNodes[p.index] || null);
    },
    Remove(p) {
      const el = byId(p.id);
      if (el) el.remove();
    },
  };

  // ---------- Contraste de color (WCAG) ----------
  function parseColor(s) {
    const m = /^rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)(?:\s*[,/]\s*([\d.]+%?))?\s*\)$/.exec(s || '');
    if (!m) return null; // formatos modernos (oklch, color-mix…): no se pueden medir aquí
    const a = m[4] === undefined ? 1 : m[4].endsWith('%') ? parseFloat(m[4]) / 100 : parseFloat(m[4]);
    return [Number(m[1]), Number(m[2]), Number(m[3]), a];
  }
  const channel = (c) => { c /= 255; return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4); };
  const luminance = (c) => 0.2126 * channel(c[0]) + 0.7152 * channel(c[1]) + 0.0722 * channel(c[2]);
  const over = (top, under) => {
    const a = top[3];
    return [top[0] * a + under[0] * (1 - a), top[1] * a + under[1] * (1 - a), top[2] * a + under[2] * (1 - a), 1];
  };
  const hex = (c) => '#' + c.slice(0, 3).map((v) => Math.round(v).toString(16).padStart(2, '0')).join('');

  // Color de fondo bajo un elemento: las capas semitransparentes se componen hasta la
  // primera opaca (o el blanco del lienzo). `null` si hay imagen o degradado: no se sabe.
  function backdrop(el) {
    const layers = [];
    for (let e = el; e; e = e.parentElement) {
      const cs = getComputedStyle(e);
      if (cs.backgroundImage && cs.backgroundImage !== 'none') return null;
      const c = parseColor(cs.backgroundColor);
      if (!c) return null;
      if (c[3] > 0) layers.push(c);
      if (c[3] >= 1) break;
    }
    let base = [255, 255, 255, 1];
    for (let i = layers.length - 1; i >= 0; i--) base = over(layers[i], base);
    return base;
  }

  const hasOwnText = (el) =>
    Array.from(el.childNodes).some((n) => n.nodeType === Node.TEXT_NODE && n.textContent.trim() !== '');

  function measureContrast() {
    const items = [];
    let checked = 0, skipped = 0;
    for (const el of document.querySelectorAll('[data-tk-id]')) {
      if (!hasOwnText(el)) continue;
      const cs = getComputedStyle(el);
      if (cs.display === 'none' || cs.visibility === 'hidden') continue;
      const r = el.getBoundingClientRect();
      if (r.width <= 0 || r.height <= 0) continue; // <script>, <style>, ocultos
      const fg = parseColor(cs.color), bg = backdrop(el);
      if (!fg || !bg) { skipped++; continue; }
      const shown = fg[3] < 1 ? over(fg, bg) : fg;
      const hi = Math.max(luminance(shown), luminance(bg)), lo = Math.min(luminance(shown), luminance(bg));
      const ratio = (hi + 0.05) / (lo + 0.05);
      const px = parseFloat(cs.fontSize), bold = parseInt(cs.fontWeight, 10) >= 700;
      const large = px >= 24 || (bold && px >= 18.66);
      checked++;
      if (ratio < (large ? 3 : 4.5) && items.length < 200) {
        items.push({ id: idOf(el), ratio: Math.round(ratio * 100) / 100, fg: hex(shown), bg: hex(bg), large });
      }
    }
    send({ type: 'Contrast', items, checked, skipped });
  }

  window.__tk = {
    // Mide el contraste de cada texto con su fondo y responde con `Contrast`.
    contrast: measureContrast,
    apply(p) {
      if (patches[p.type]) patches[p.type](p);
      ensureCanvasBackground(); // los cambios de estilo pueden añadir o quitar fondos
      highlight();
    },
    // Trae el elemento a la vista si no se ve (la selección vino del código, no de un clic).
    reveal(id) {
      const el = byId(id);
      const box = el && visibleBox(el);
      if (box) box.el.scrollIntoView({ block: 'nearest', inline: 'nearest' });
      highlight(); // el desplazamiento mueve el elemento: se recoloca el recuadro
    },
    // Vuelve a pedir las hojas de estilo externas (el host sirve la versión en edición)
    // sin recargar la página: se conserva el scroll y la selección.
    refreshCss() {
      const v = String(Date.now());
      document.querySelectorAll('link[rel~="stylesheet"]').forEach((link) => {
        try {
          const u = new URL(link.href);
          u.searchParams.set('tkv', v);
          link.href = u.toString();
        } catch (_) { /* href no válido: se deja como está */ }
      });
    },
    // ¿Qué reglas CSS afectan a este elemento? Lo decide el navegador: así cuentan
    // combinadores, pseudoclases y las condiciones de @media tal como las ve la página.
    matchRules(id, rules) {
      const el = byId(id);
      const out = rules.map((r) => {
        if (!el) return false;
        try {
          if (r.context && /^@media/i.test(r.context) &&
              !matchMedia(r.context.replace(/^@media\s*/i, '')).matches) return false;
          return el.matches(r.selector);
        } catch (_) {
          return false; // selector que el navegador no entiende (p. ej. un pseudo-elemento)
        }
      });
      send({ type: 'Matches', id, rules: out });
    },
  };

  // ---------- Arrastrar elementos ----------
  // El arrastre solo dibuja (fantasma, cursor) y pregunta a Rust; el documento no cambia hasta
  // soltar en un sitio válido. Si no lo es, no hay nada que deshacer: sigue donde estaba.
  let drag = null; // { id, x, y, active, target, place, ghost }
  let dropOk = false;
  let dropWhy = '';
  let suppressClick = false;

  function dragTarget(x, y) {
    for (const el of document.elementsFromPoint(x, y)) {
      if (el.closest('[data-tk-overlay]')) continue;
      const t = el.closest('[data-tk-id]');
      if (t) return t;
    }
    return null;
  }

  const hasBox = (el) => {
    if (el.hasAttribute('data-tk-overlay')) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  };
  const isEmpty = (el) =>
    !Array.from(el.childNodes).some((n) => n.nodeType === 1 || (n.nodeType === 3 && n.textContent.trim() !== ''));

  // Respecto a qué elemento y de qué lado se suelta en (x, y). Mitades: la superior es «antes» y la
  // inferior «después». «Dentro» solo en elementos vacíos (su zona central). Si se apunta a un
  // contenedor con hijos (su relleno, o el hueco entre dos bloques), vale el hijo más cercano en
  // vertical: antes o después según de qué lado de su mitad se esté.
  function dropAt(t, y) {
    if (t.tagName.toLowerCase() === 'html' && document.body) t = document.body;
    const kids = Array.from(t.children).filter((k) => k.hasAttribute('data-tk-id') && hasBox(k));
    if (kids.length) {
      const dist = (k) => {
        const r = k.getBoundingClientRect();
        return y < r.top ? r.top - y : y > r.bottom ? y - r.bottom : 0;
      };
      const c = kids.reduce((a, b) => (dist(b) < dist(a) ? b : a));
      const r = c.getBoundingClientRect();
      return { el: c, place: y < r.top + r.height / 2 ? 'before' : 'after' };
    }
    const tag = t.tagName.toLowerCase();
    if (tag === 'body') return { el: t, place: 'inside' };
    const r = t.getBoundingClientRect();
    const f = r.height > 0 ? (y - r.top) / r.height : 0.5;
    if (isEmpty(t) && f >= 0.25 && f <= 0.75) return { el: t, place: 'inside' };
    return { el: t, place: f < 0.5 ? 'before' : 'after' };
  }

  // Mientras se arrastra se enciende solo el borde de los elementos que enmarcan el hueco donde
  // caería: el anterior y el siguiente (o el propio contenedor, si se suelta dentro de uno vacío).
  const GUIDE_CSS =
    '[data-tk-hover]{outline:1px dashed #3b82f6!important;outline-offset:-1px!important}';
  let guides = null;
  let lit = [];

  function setGuides(on) {
    if (on && !guides) {
      guides = document.createElement('style');
      guides.setAttribute('data-tk-overlay', '');
      guides.textContent = GUIDE_CSS;
      document.documentElement.appendChild(guides);
    } else if (!on && guides) {
      guides.remove();
      guides = null;
    }
    if (!on) setLit([]);
  }

  function setLit(list) {
    lit.forEach((el) => { if (!list.includes(el)) el.removeAttribute('data-tk-hover'); });
    list.forEach((el) => el.setAttribute('data-tk-hover', ''));
    lit = list;
  }

  // Los elementos que enmarcan el hueco: el hermano anterior y el posterior (sin contar el que
  // se arrastra ni los que no tienen caja).
  function neighbours(t, place, dragged) {
    if (place === 'inside') return [t];
    const skip = (e) => e === dragged || !e.hasAttribute('data-tk-id') || !hasBox(e);
    const prevOf = (e) => { let n = e.previousElementSibling; while (n && skip(n)) n = n.previousElementSibling; return n; };
    const nextOf = (e) => { let n = e.nextElementSibling; while (n && skip(n)) n = n.nextElementSibling; return n; };
    const list = t === dragged ? [prevOf(t), nextOf(t)]
      : place === 'before' ? [prevOf(t), t] : [t, nextOf(t)];
    return list.filter((e) => e && !(dragged && dragged.contains(e)));
  }

  function setDragCursor() {
    const on = drag && drag.active;
    document.documentElement.style.cursor = !on ? '' : dropOk ? 'grabbing' : 'not-allowed';
    if (drag && drag.ghost) {
      drag.ghost.style.background = dropOk ? '#3b82f6' : '#dc2626';
      drag.ghost.textContent = (dropOk || !dropWhy ? '' : '⛔ ') + drag.name + (dropOk || !dropWhy ? '' : ' — ' + dropWhy);
    }
  }

  function startDrag(e) {
    if (e.button !== 0 || selected == null || editing) return;
    const el = byId(selected);
    const tag = el && el.tagName.toLowerCase();
    if (!el || tag === 'html' || tag === 'body' || tag === 'head') return;
    e.preventDefault();
    e.stopPropagation();
    drag = { id: selected, x: e.clientX, y: e.clientY, active: false, target: null, place: null, ghost: null, name: describe(el) };
    dropOk = false;
    dropWhy = '';
  }

  function finishDrag(send_) {
    const d = drag;
    drag = null;
    dropOk = false;
    dropWhy = '';
    document.documentElement.style.cursor = '';
    setGuides(false);
    if (d && d.ghost) d.ghost.remove();
    if (d && d.active) {
      suppressClick = true; // el clic que sigue al soltar no debe cambiar la selección
      setTimeout(() => { suppressClick = false; }, 80);
      if (send_ && d.target) {
        send({ type: 'MoveElement', id: d.id, target: idOf(d.target), place: d.place });
      } else {
        send({ type: 'DragEnd' });
      }
    }
  }

  document.addEventListener('mousemove', (e) => {
    if (!drag) return;
    if (!drag.active) {
      if (Math.hypot(e.clientX - drag.x, e.clientY - drag.y) < 4) return;
      drag.active = true;
      const g = document.createElement('div');
      g.setAttribute('data-tk-overlay', '');
      g.style.cssText =
        'position:fixed;pointer-events:none;z-index:2147483647;padding:1px 8px;border-radius:3px;' +
        'font:11px/16px system-ui,sans-serif;color:#fff;background:#3b82f6;white-space:nowrap;' +
        'box-shadow:0 2px 8px rgba(0,0,0,.35);opacity:.92;';
      document.documentElement.appendChild(g);
      drag.ghost = g;
      setGuides(true);
      setDragCursor();
    }
    e.preventDefault();
    drag.ghost.style.left = e.clientX + 14 + 'px';
    drag.ghost.style.top = e.clientY + 14 + 'px';
    const hit = dragTarget(e.clientX, e.clientY);
    if (!hit) {
      drag.target = null;
      drag.place = null;
      dropOk = false;
      dropWhy = '';
      setLit([]);
      setDragCursor();
      return;
    }
    const { el: t, place } = dropAt(hit, e.clientY);
    if (t !== drag.target || place !== drag.place) {
      drag.target = t;
      drag.place = place;
      setLit(neighbours(t, place, byId(drag.id)));
      send({ type: 'DragOver', id: drag.id, target: idOf(t), place });
    }
  }, true);

  document.addEventListener('mouseup', (e) => {
    if (!drag) return;
    if (drag.active) { e.preventDefault(); e.stopPropagation(); }
    finishDrag(true);
  }, true);

  // ---------- Edición de texto en línea ----------
  function isTextOnly(el) {
    return !VOID.has(el.tagName.toLowerCase()) &&
      Array.from(el.childNodes).every((n) => n.nodeType === Node.TEXT_NODE);
  }

  function startEdit(el) {
    if (editing || !isTextOnly(el)) return;
    editing = { el, original: el.textContent };
    placeMark(); // oculta el cursor de inserción durante la edición
    el.contentEditable = 'plaintext-only';
    if (el.contentEditable !== 'plaintext-only') el.contentEditable = 'true';
    el.focus();
    const range = document.createRange();
    range.selectNodeContents(el);
    const sel = getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
    el.addEventListener('blur', commitEdit, { once: true });
  }

  function endEdit() {
    const { el } = editing;
    el.removeAttribute('contenteditable');
    editing = null;
  }

  function commitEdit() {
    if (!editing) return;
    const { el, original } = editing;
    const text = el.textContent;
    endEdit();
    if (text !== original) send({ type: 'EditText', id: idOf(el), text });
    highlight();
  }

  function cancelEdit() {
    if (!editing) return;
    const { el, original } = editing;
    el.removeEventListener('blur', commitEdit);
    el.textContent = original;
    endEdit();
    highlight();
  }

  // ---------- Entrada ----------
  // Fase de captura: gana a los handlers del propio sitio (enlaces, botones).
  document.addEventListener('click', (e) => {
    if (suppressClick) { e.preventDefault(); e.stopPropagation(); return; } // fin de un arrastre
    if (editing && editing.el.contains(e.target)) return; // posicionar el cursor
    e.preventDefault();
    e.stopPropagation();
    const el = e.target.closest && e.target.closest('[data-tk-id]');
    if (el) send({ type: 'Select', id: idOf(el) });
  }, true);

  document.addEventListener('dblclick', (e) => {
    const el = e.target.closest && e.target.closest('[data-tk-id]');
    if (el) { e.preventDefault(); startEdit(el); }
  }, true);

  document.addEventListener('submit', (e) => e.preventDefault(), true);

  document.addEventListener('keydown', (e) => {
    if (drag && e.key === 'Escape') { e.preventDefault(); finishDrag(false); return; } // cancelar
    if (editing) {
      if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); editing.el.blur(); }
      else if (e.key === 'Escape') { e.preventDefault(); cancelEdit(); }
      return; // el resto, incluido Ctrl+Z, es del campo de texto
    }
    const mod = e.ctrlKey || e.metaKey;
    const k = e.key.toLowerCase();
    let msg = null;
    if (mod && k === 'z') msg = e.shiftKey ? 'Redo' : 'Undo';
    else if (mod && k === 'y') msg = 'Redo';
    // Copiar y pegar elementos; si hay texto seleccionado en la página, Ctrl+C sigue siendo el nativo.
    else if (mod && k === 'c' && getSelection().toString() === '') msg = 'Copy';
    else if (mod && k === 'v') msg = 'Paste';
    else if (mod && k === 's') msg = e.shiftKey ? 'SaveAs' : 'Save';
    else if (mod && k === 'o') msg = 'Open';
    else if (e.key === 'Delete') msg = 'Delete';
    if (msg) { e.preventDefault(); send({ type: msg }); }
  }, true);

  // Cualquier cambio de maquetación (imágenes que cargan, redimensionar) recoloca el recuadro.
  new ResizeObserver(highlight).observe(document.documentElement);
  window.addEventListener('resize', highlight);

  function ready() {
    ensureCanvasBackground();
    send({ type: 'Ready' });
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', ready);
  else ready();
})();
