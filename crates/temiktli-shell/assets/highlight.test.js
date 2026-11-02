// Pruebas del coloreador. Ejecutar con: node crates/temiktli-shell/assets/highlight.test.js
'use strict';
const assert = require('node:assert/strict');
const { highlight } = require('./highlight.js');

const textOf = (h) =>
  h.replace(/<[^>]*>/g, '').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&');

// 1. Invariante: colorear nunca cambia el texto, ni siquiera con basura.
const samples = [
  '',
  '<',
  '<<<>>>',
  '<a href="x>texto</a>',
  '<a title="a>b<c" href=x>e</a>',
  "<!DOCTYPE html>\n<!-- c --><html lang=es><body class='a'>&copy; &x &#169; &amp</body></html>",
  '<style>\n a { color: #fff; margin: 0 auto !important }\n@media (max-width: 600px) { .x { width: 50% } }\n/* c */</style>',
  '<script>const a = "x</div>"; // nota\nif (a<b) { f(1.5e3); }</script>',
  '<style>a{',
  '<script>`sin cerrar',
  '<p\n  id = "x"\n  hidden\n>\r\n\ttexto</p>',
  '<img src=a.png alt="">',
  'a < b && c > d',
  '<svg><path d="M0 0"/><circle r=1 /></svg>',
];
for (const s of samples) assert.equal(textOf(highlight(s)), s, JSON.stringify(s));

// 2. Texto aleatorio con muchos caracteres especiales.
const alphabet = ['<', '>', '/', '"', "'", '=', '&', ';', '!', '-', '{', '}', ':', '@', '#', ' ', '\n', 'a', 'b', 'style', 'script', '</'];
let seed = 42;
const rnd = () => (seed = (seed * 1664525 + 1013904223) >>> 0) / 2 ** 32;
for (let n = 0; n < 500; n++) {
  let s = '';
  for (let k = 0, len = Math.floor(rnd() * 60); k < len; k++) s += alphabet[Math.floor(rnd() * alphabet.length)];
  assert.equal(textOf(highlight(s)), s, JSON.stringify(s));
}

// 3. Colores: cada pieza recibe su clase.
const h = highlight('<a href="x" hidden>Hola &amp; adiós</a><!-- n -->');
assert.match(h, /<span class="c-tag">a<\/span>/);
assert.match(h, /<span class="c-attr">href<\/span>/);
assert.match(h, /<span class="c-str">"x"<\/span>/);
assert.match(h, /<span class="c-ent">&amp;amp;<\/span>/);
assert.match(h, /<span class="c-comment">&lt;!-- n --&gt;<\/span>/);

const c = highlight('<style>p, .x { color: #fff; margin: 0 8px !important }\n@media (max-width: 600px) { h1 { top: 1px } }</style>');
assert.match(c, /<span class="c-sel">p, \.x <\/span>/);
assert.match(c, /<span class="c-prop">color<\/span>/);
assert.match(c, /<span class="c-num">#fff<\/span>/);
assert.match(c, /<span class="c-num">8px<\/span>/);
assert.match(c, /<span class="c-at">@media<\/span>/);
assert.match(c, /<span class="c-sel"> h1 <\/span>/); // dentro de @media también hay selectores

const j = highlight('<script>const a = "s"; // n\n</script>');
assert.match(j, /<span class="c-kw">const<\/span>/);
assert.match(j, /<span class="c-comment">\/\/ n<\/span>/);

// 4. El contenido de <script> no se interpreta como HTML.
assert.ok(!/c-tag">div/.test(highlight('<script>x = "</div>";</script>')));

// 5. Hoja de estilo suelta (.css): mismos colores, sin pasar por HTML.
const { highlightCss } = require('./highlight.js');
const sheet = '/* base */\n@media (max-width: 600px) {\n  .a, #b > c { color: #fff; margin: 0 8px !important }\n}\n';
assert.equal(textOf(highlightCss(sheet)), sheet);
assert.match(highlightCss(sheet), /<span class="c-prop">color<\/span>/);
assert.match(highlightCss(sheet), /<span class="c-at">@media<\/span>/);
assert.match(highlightCss('a < b { c: "x<y" }'), /<span class="c-str">"x&lt;y"<\/span>/);
for (let n = 0; n < 200; n++) {
  let s = '';
  for (let k = 0, len = Math.floor(rnd() * 50); k < len; k++) s += alphabet[Math.floor(rnd() * alphabet.length)];
  assert.equal(textOf(highlightCss(s)), s, JSON.stringify(s));
}

console.log('highlight: ok');
