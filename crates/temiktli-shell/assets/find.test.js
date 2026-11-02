// Pruebas de buscar y reemplazar. Ejecutar con: node crates/temiktli-shell/assets/find.test.js
'use strict';
const assert = require('node:assert/strict');
const F = require('./find.js');

const find = (text, query, opts) => {
  const { re, error } = F.compile(query, opts);
  assert.equal(error, null, String(error));
  return F.findAll(text, re).matches.map((m) => text.slice(m.start, m.end));
};

// --- Búsqueda literal ---
assert.deepEqual(find('Hola hola HOLA', 'hola'), ['Hola', 'hola', 'HOLA'], 'sin distinguir mayúsculas');
assert.deepEqual(find('Hola hola HOLA', 'hola', { caseSensitive: true }), ['hola']);
assert.deepEqual(find('a.b a+b (a)', 'a.b'), ['a.b'], 'el punto es literal');
assert.deepEqual(find('1+1=2 y 1+1', '1+1'), ['1+1', '1+1'], 'los símbolos de regex son literales');
assert.deepEqual(find('x[0] y x[1]', 'x[0]'), ['x[0]']);
assert.deepEqual(find('<div class="a">', '<div class="a">'), ['<div class="a">']);
assert.deepEqual(find('a\\b', 'a\\b'), ['a\\b']);
assert.deepEqual(find('', 'x'), []);
assert.deepEqual(F.findAll('abc', null).matches, []);
assert.deepEqual(F.compile('', {}), { re: null, error: null });

// --- Expresiones regulares ---
assert.deepEqual(find('a1 b22 c333', '\\d+', { regex: true }), ['1', '22', '333']);
assert.deepEqual(find('línea1\nlínea2', '^línea\\d$', { regex: true }), ['línea1', 'línea2'], '^ y $ por línea');
assert.deepEqual(find('aaa', 'a*', { regex: true }), ['aaa'], 'las coincidencias vacías se omiten');
assert.deepEqual(find('abc', 'x*', { regex: true }), [], 'solo vacías: nada');
const bad = F.compile('(', { regex: true });
assert.equal(bad.re, null);
assert.ok(bad.error && bad.error.length > 0, 'una regex inválida da un mensaje, no una excepción');

// --- Palabra completa (con letras Unicode) ---
assert.deepEqual(find('casa casas casar la casa', 'casa', { word: true }), ['casa', 'casa']);
assert.deepEqual(find('niño niños', 'niño', { word: true }), ['niño'], 'la ñ cuenta como letra');
assert.deepEqual(find('año1 año', 'año', { word: true }), ['año'], 'los dígitos pegados no son palabra');
assert.deepEqual(find('un_div div', 'div', { word: true }), ['div'], 'el guion bajo une palabras');
assert.deepEqual(find('red-red', 'red', { word: true }), ['red', 'red'], 'el guion medio separa');
assert.deepEqual(find('a.b', 'a.b', { word: true }), ['a.b']);

// --- Límite de resultados ---
const many = 'x'.repeat(6000);
const lim = F.findAll(many, F.compile('x', {}).re);
assert.equal(lim.matches.length, F.MAX_MATCHES);
assert.equal(lim.truncated, true);
assert.equal(F.findAll('xx', F.compile('x', {}).re).truncated, false);

// --- Reemplazar ---
const rep = (text, q, r, opts) => F.replaceAll(text, F.compile(q, opts).re, r, !!(opts && opts.regex));
assert.deepEqual(rep('a b a', 'a', 'X'), { text: 'X b X', count: 2 });
assert.deepEqual(rep('a b a', 'z', 'X'), { text: 'a b a', count: 0 });
assert.deepEqual(rep('Hola hola', 'hola', 'adiós'), { text: 'adiós adiós', count: 2 });
assert.deepEqual(rep('a b', 'a', ''), { text: ' b', count: 1 }, 'se puede reemplazar por nada');
assert.deepEqual(rep('precio $5', '$5', '$10'), { text: 'precio $10', count: 1 }, 'el $ es literal');
assert.deepEqual(rep('a', 'a', '$1$&$$'), { text: '$1$&$$', count: 1 }, 'en modo normal «$…» es literal');
assert.deepEqual(rep('2024-05-06', '(\\d+)-(\\d+)-(\\d+)', '$3/$2/$1', { regex: true }), { text: '06/05/2024', count: 1 });
assert.deepEqual(rep('ab', '(a)(b)', '$2$&', { regex: true }), { text: 'bab', count: 1 });
assert.deepEqual(rep('a1 b2', '(?<l>[a-z])\\d', '$<l>', { regex: true }), { text: 'a b', count: 2 });
assert.deepEqual(rep('aaa', 'a*', 'X', { regex: true }), { text: 'X', count: 1 });
assert.deepEqual(rep('casa casas', 'casa', 'hogar', { word: true }), { text: 'hogar casas', count: 1 },
  'reemplazar palabra completa respeta el contexto');
assert.deepEqual(rep('<p>x</p>', '<p>', '<div>'), { text: '<div>x</p>', count: 1 });

// Una sola coincidencia (el botón «Reemplazar»).
const t = 'ab ab';
const re = F.compile('(a)b', { regex: true }).re;
assert.equal(F.replacementAt(t, 3, re, '[$1]', true), '[a]');
assert.equal(F.replacementAt(t, 1, re, '[$1]', true), null, 'donde no hay coincidencia, nada');

// --- Navegación ---
const ms = [{ start: 2, end: 4 }, { start: 10, end: 12 }, { start: 20, end: 22 }];
assert.equal(F.stepFrom(ms, 0, 1), 0);
assert.equal(F.stepFrom(ms, 5, 1), 1);
assert.equal(F.stepFrom(ms, 25, 1), 0, 'da la vuelta al final');
assert.equal(F.stepFrom(ms, 25, -1), 2);
assert.equal(F.stepFrom(ms, 11, -1), 0, 'la anterior termina antes del cursor');
assert.equal(F.stepFrom(ms, 0, -1), 2, 'da la vuelta al principio');
assert.equal(F.stepFrom([], 0, 1), -1);

// --- Marcas: el texto visible no cambia ---
const text = 'a <b> & c <b>';
const marks = F.marksHtml(text, F.findAll(text, F.compile('<b>', {}).re).matches, 1);
assert.equal(marks.replace(/<\/?mark[^>]*>/g, '').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&'), text);
assert.match(marks, /<mark class="m">&lt;b&gt;<\/mark>/);
assert.match(marks, /<mark class="cur">&lt;b&gt;<\/mark>/);

// --- Línea y columna ---
assert.deepEqual(F.lineCol('abc\n  de', 6), { line: 1, col: 2 });
assert.deepEqual(F.lineCol('\tx', 1), { line: 0, col: 2 });
assert.deepEqual(F.lineCol('', 0), { line: 0, col: 0 });

// --- Robustez: nunca falla con texto y consultas arbitrarios ---
const alphabet = ['<', '>', '/', '"', '$', '(', ')', '[', ']', '\\', '*', '+', '?', '.', 'a', 'ñ', ' ', '\n', '|', '^', '{', '}'];
let seed = 7;
const rnd = () => (seed = (seed * 1664525 + 1013904223) >>> 0) / 2 ** 32;
const rs = (n) => Array.from({ length: Math.floor(rnd() * n) }, () => alphabet[Math.floor(rnd() * alphabet.length)]).join('');
for (let i = 0; i < 400; i++) {
  const text2 = rs(40), query = rs(6);
  for (const opts of [{}, { regex: true }, { word: true }, { regex: true, word: true, caseSensitive: true }]) {
    const { re } = F.compile(query, opts);
    if (!re) continue;
    const { matches } = F.findAll(text2, re);
    F.marksHtml(text2, matches, 0);
    const out = F.replaceAll(text2, re, '$&·', !!opts.regex);
    assert.equal(out.count, matches.length);
  }
}

console.log('find: ok');
