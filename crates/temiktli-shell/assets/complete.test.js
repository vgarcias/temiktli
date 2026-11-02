// Pruebas del autocompletado. Ejecutar con: node crates/temiktli-shell/assets/complete.test.js
// En los textos, «|» marca la posición del cursor.
'use strict';
const assert = require('node:assert/strict');
const { analyze, suggest, closingFor } = require('./complete.js');

function ctx(marked) {
  const pos = marked.indexOf('|');
  assert.ok(pos >= 0, 'falta el cursor en ' + marked);
  const text = marked.slice(0, pos) + marked.slice(pos + 1);
  return { text, pos, a: analyze(text, pos) };
}
const labels = (marked, force) => suggest(ctx(marked).a, force).map((s) => s.label);

// --- Etiquetas ---
assert.equal(ctx('<d|').a.kind, 'tag');
assert.equal(ctx('<d|').a.prefix, 'd');
assert.ok(labels('<d|').includes('div'));
assert.ok(!labels('<d|').includes('a'));
assert.ok(labels('<|').length > 5, 'tras «<» se ofrecen etiquetas');
assert.equal(labels('<div|').includes('div'), false, 'lo ya escrito entero no se vuelve a ofrecer');

// --- Cierre ---
assert.deepEqual(labels('<div><p>x</|'), ['</p>']);
assert.deepEqual(labels('<div><p>x</p></|'), ['</div>']);
assert.deepEqual(labels('<ul><li>a<li>b</|').length, 1);
assert.deepEqual(labels('<br><hr></|'), [], 'las etiquetas vacías no quedan abiertas');
const close = suggest(ctx('<div></|').a)[0];
assert.equal(close.insert, 'div>');

// --- Atributos ---
assert.equal(ctx('<a hr|').a.kind, 'attr');
assert.ok(labels('<a hr|').includes('href'));
assert.equal(suggest(ctx('<a hr|').a).find((s) => s.label === 'href').insert, 'href=""');
assert.equal(suggest(ctx('<a hr|').a).find((s) => s.label === 'href').back, 1);
assert.equal(suggest(ctx('<input dis|').a).find((s) => s.label === 'disabled').insert, 'disabled');
assert.deepEqual(labels('<a href="x" |'), [], 'sin letras no se abre la lista');
assert.ok(labels('<a href="x" |', true).length > 5, 'Ctrl+Espacio la fuerza');
assert.ok(labels('<img sr|').includes('src') && labels('<img sr|').includes('srcset'));
assert.ok(!labels('<div sr|').includes('src'), 'los atributos dependen de la etiqueta');
assert.equal(ctx('<a title="a>b" cl|').a.kind, 'attr', 'un «>» entre comillas no cierra la etiqueta');
assert.equal(ctx('<a href=|').a.kind, 'attrvalue');
assert.equal(ctx('<a href="x|').a.kind, 'quote');
assert.equal(ctx('<a href="x|').a.quote, '"');
assert.equal(ctx('<a href=abc|').a.kind, 'attrvalue', 'un valor sin comillas sigue siendo un valor');
assert.deepEqual(labels('<a href=abc|'), [], 'y no ofrece sugerencias');

// --- CSS dentro de <style> ---
assert.equal(ctx('<style>p { col|').a.kind, 'css-prop');
assert.ok(labels('<style>p { col|').includes('color'));
assert.equal(suggest(ctx('<style>p { col|').a).find((s) => s.label === 'color').insert, 'color: ;');
assert.equal(ctx('<style>p { color: re|').a.kind, 'none', 'en un valor no se sugieren propiedades');
assert.equal(ctx('<style>p { color: red; wi|').a.kind, 'css-prop');
assert.equal(ctx('<style>@me|').a.kind, 'css-at');
assert.deepEqual(labels('<style>@me|'), ['@media']);
assert.equal(ctx('<style>@media (x) { .a { wi|').a.kind, 'css-prop');
assert.equal(ctx('<style>@media (x) { ba|').a.kind, 'none', 'dentro de @media se escriben selectores');
assert.equal(ctx('<style>@media (x) { @su|').a.kind, 'css-at');
assert.equal(ctx('<style>/* col|').a.kind, 'none');
assert.equal(ctx('<style>p { content: "a{ co|').a.kind, 'none');
assert.equal(ctx('<style>p { color: red }\n</style><d|').a.kind, 'tag', 'tras </style> vuelve el HTML');
const media = suggest(ctx('<style>@me|').a)[0];
assert.ok(media.insert.startsWith('@media (max-width: 600px) {') && media.back === 2);

// --- Sitios donde no se sugiere nada ---
assert.equal(ctx('<script>if (a<b) { x|').a.kind, 'none');
assert.equal(ctx('<!-- <d|').a.kind, 'none');
assert.equal(ctx('<p>texto|').a.kind, 'none');
assert.equal(ctx('a < b|').a.kind, 'none');

// --- Cierre automático al escribir «>» ---
const closing = (marked) => { const c = ctx(marked); return closingFor(c.text, c.pos); };
assert.equal(closing('<div>|'), '</div>');
assert.equal(closing('<section class="a">|'), '</section>');
assert.equal(closing('<DIV>|'), '</div>');
assert.equal(closing('<style>|'), '</style>');
assert.equal(closing('<br>|'), null);
assert.equal(closing('<img src=x>|'), null);
assert.equal(closing('<img src=x/>|'), null);
assert.equal(closing('<div>|</div>'), null, 'si ya está cerrada no se duplica');
assert.equal(closing('<p>a</p>|'), null);
assert.equal(closing('<a title="x>y">|'), '</a>');

// --- Hoja de estilo suelta (modo css) ---
const cssCtx = (marked) => {
  const pos = marked.indexOf('|');
  const text = marked.slice(0, pos) + marked.slice(pos + 1);
  return analyze(text, pos, { css: true });
};
assert.equal(cssCtx('p { col|').kind, 'css-prop');
assert.ok(suggest(cssCtx('p { col|')).some((s) => s.label === 'color'));
assert.equal(cssCtx('@me|').kind, 'css-at');
assert.equal(cssCtx('@media (x) { .a { wi|').kind, 'css-prop');
assert.equal(cssCtx('p { color: re|').kind, 'none');
assert.equal(cssCtx('<d|').kind, 'none', 'en un .css «<» no es una etiqueta');
assert.equal(cssCtx('|').kind, 'none');

// --- Robustez: nunca falla, en ninguna posición, con texto roto ---
const junk = [
  '', '<', '<<<>>>', '<a', '<a b="', '</', '<!--', '<style>', '<style>a{', '<script>`x',
  '<div class="a>texto</div>', '<p\n  id = "x"\n  hidden\n>\r\n\ttexto', '<style>@media{{{}}}',
  '<style>/* sin cerrar', 'a < b && c > d', '\u{1F600}<p>\u{1F600}</p>',
];
for (const s of junk) {
  for (let p = 0; p <= s.length; p++) {
    const a = analyze(s, p);
    suggest(a, true);
    closingFor(s, p);
    assert.ok(a.from >= 0 && a.from <= p, JSON.stringify([s, p, a.from]));
  }
}

console.log('complete: ok');
