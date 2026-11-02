// Pruebas de la traducción de los webviews. Ejecutar con: node crates/temiktli-shell/assets/i18n.test.js
'use strict';
const assert = require('node:assert/strict');
const { createI18n, fill } = require('./i18n.js');

const dict = {
  '$meta.language': 'es',
  '$meta.name': 'Español',
  '$languages': [{ code: 'en', name: 'English' }, { code: 'es', name: 'Español' }],
  'menu.file.open': 'Abrir…',
  'status.saved': 'Guardado: {path}',
  'menu.search.hits': '{hits} en {files}',
  'units.file_one': '{n} archivo',
  'units.file_other': '{n} archivos',
  'lint.poor_alt_words': ['imagen', 'foto'],
};
const i18n = createI18n(dict);
const { t, tn } = i18n;

// --- t: con y sin argumentos ---
assert.equal(t('menu.file.open'), 'Abrir…');
assert.equal(t('status.saved', { path: 'C:\\a.html' }), 'Guardado: C:\\a.html');
assert.equal(t('status.saved', { path: 0 }), 'Guardado: 0', 'los números se escriben');
assert.equal(t('menu.search.hits', { hits: 3, files: tn('units.file', 2) }), '3 en 2 archivos', 'anidado');

// --- clave ausente → [clave] ---
assert.equal(t('no.existe'), '[no.existe]');
assert.equal(t('no.existe', { x: 1 }), '[no.existe]');
assert.equal(t('lint.poor_alt_words'), '[lint.poor_alt_words]', 'una lista no es un texto');

// --- marcador no provisto: se conserva ---
assert.equal(t('status.saved'), 'Guardado: {path}');
assert.equal(t('status.saved', {}), 'Guardado: {path}');
assert.equal(t('status.saved', { other: 1 }), 'Guardado: {path}');
assert.equal(t('status.saved', { path: undefined }), 'Guardado: {path}');
assert.equal(fill('{a} y {b}', { a: 'x' }), 'x y {b}');
assert.equal(fill('{a}{a}', { a: '$&' }), '$&$&', 'el valor es literal, no un patrón de replace');

// --- tn: singular y plural ---
assert.equal(tn('units.file', 1), '1 archivo');
assert.equal(tn('units.file', 0), '0 archivos');
assert.equal(tn('units.file', 2), '2 archivos');
assert.equal(tn('units.file', 25), '25 archivos');
assert.equal(tn('units.nada', 1), '[units.nada_one]');
assert.equal(tn('units.nada', 3), '[units.nada_other]');

// --- metadatos ---
assert.equal(i18n.language, 'es');
assert.equal(createI18n({}).language, 'en', 'sin diccionario, inglés');
assert.deepEqual(i18n.dict['$languages'].map((l) => l.code), ['en', 'es']);
assert.equal(typeof i18n.apply, 'function');
assert.doesNotThrow(() => createI18n({}).apply(null), 'sin DOM no hace nada');

// --- el marcador que Rust sustituye sigue en el archivo, una sola vez ---
const src = require('node:fs').readFileSync(require('node:path').join(__dirname, 'i18n.js'), 'utf8');
assert.equal(src.split('/*I18N_JSON*/{}').length - 1, 1);

console.log('i18n: ok');
