// Traducción de los webviews. Rust sustituye el marcador I18N_JSON (el objeto vacío de
// `EMBEDDED`, más abajo; solo puede aparecer una vez) por el diccionario plano del idioma
// activo (claves con puntos → texto; con el inglés de respaldo ya mezclado, así nunca falta
// una clave) y pega este archivo en cada página en lugar del marcador I18N_JS, antes que
// cualquier otro script. En node (`i18n.test.js`) se construye con `createI18n(dict)`.
//
// El diccionario trae además `$meta.language` (código), `$meta.name` y `$languages`
// (lista de { code, name } con los idiomas disponibles, para el submenú Vista → Idioma).
(function () {
  'use strict';

  // Sustituye los marcadores `{nombre}` por `args[nombre]`; los no provistos se quedan tal cual.
  function fill(text, args) {
    if (!args) return text;
    return text.replace(/\{(\w+)\}/g, (m, name) =>
      Object.prototype.hasOwnProperty.call(args, name) && args[name] !== undefined ? String(args[name]) : m);
  }

  function createI18n(dict) {
    dict = dict || {};

    // Texto de una clave. Si falta, la clave entre corchetes: se ve en las pruebas, nunca se calla.
    const t = (key, args) => {
      const v = dict[key];
      return typeof v === 'string' ? fill(v, args) : '[' + key + ']';
    };

    // Plural: `clave_one` si n === 1, `clave_other` si no; `{n}` lleva el número.
    const tn = (key, n, args) => t(key + (n === 1 ? '_one' : '_other'), Object.assign({ n }, args || {}));

    // Rellena el marcado estático: [data-i18n] (texto), [data-i18n-title] y [data-i18n-placeholder].
    // Se llama una vez al cargar; los textos que cambian usan `t` / `tn`.
    const apply = (root) => {
      root = root || (typeof document !== 'undefined' ? document : null);
      if (!root) return;
      root.querySelectorAll('[data-i18n]').forEach((e) => { e.textContent = t(e.dataset.i18n); });
      root.querySelectorAll('[data-i18n-title]').forEach((e) => { e.title = t(e.dataset.i18nTitle); });
      root.querySelectorAll('[data-i18n-placeholder]').forEach((e) => { e.placeholder = t(e.dataset.i18nPlaceholder); });
      const lang = dict['$meta.language'];
      if (lang && typeof document !== 'undefined') document.documentElement.lang = lang;
    };

    return { dict, language: dict['$meta.language'] || 'en', t, tn, apply };
  }

  const EMBEDDED = /*I18N_JSON*/{};

  if (typeof module !== 'undefined' && module.exports) module.exports = { createI18n, fill };
  if (typeof window !== 'undefined') window.__i18n = createI18n(window.__i18nDict || EMBEDDED);
})();
