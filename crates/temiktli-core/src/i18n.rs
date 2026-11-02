//! Textos de la interfaz en varios idiomas. Los archivos de idioma viven en
//! `crates/temiktli-shell/locales/*.json` y se incrustan aquí (el núcleo también produce
//! mensajes: avisos de la revisión, errores de anidación…). El inglés es el respaldo: una
//! clave que falte en el idioma activo se busca en él, y si tampoco está se devuelve `[clave]`
//! para que un test lo detecte antes que el usuario.
//!
//! Claves con puntos (`menu.file.open`), marcadores con nombre (`{path}`) y plurales con dos
//! claves (`units.file_one` / `units.file_other`). Las listas (`lint.vague_link_words`) se
//! unen entre todos los idiomas: miran el contenido de la página, no la interfaz.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::{LazyLock, RwLock};

/// Idiomas incrustados: (código, JSON). Es la única lista: de ella salen el selector del
/// menú, la detección del idioma del sistema y el respaldo.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", include_str!("../../temiktli-shell/locales/en.json")),
    ("es", include_str!("../../temiktli-shell/locales/es.json")),
    ("fr", include_str!("../../temiktli-shell/locales/fr.json")),
    ("pt", include_str!("../../temiktli-shell/locales/pt.json")),
    ("de", include_str!("../../temiktli-shell/locales/de.json")),
    ("it", include_str!("../../temiktli-shell/locales/it.json")),
    ("pl", include_str!("../../temiktli-shell/locales/pl.json")),
];

/// Idioma por omisión y de respaldo.
pub const DEFAULT: &str = "en";

/// Un idioma cargado: sus textos y sus listas, con las claves aplanadas (`a.b.c`).
#[derive(Debug, Clone)]
pub struct Dict {
    code: String,
    strings: BTreeMap<String, String>,
    lists: BTreeMap<String, Vec<String>>,
}

impl Dict {
    pub fn parse(json: &str) -> Result<Dict, String> {
        let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let mut d = Dict { code: String::new(), strings: BTreeMap::new(), lists: BTreeMap::new() };
        flatten(&value, "", &mut d)?;
        d.code = d.strings.get("$meta.language").cloned().ok_or("falta $meta.language")?;
        Ok(d)
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    /// Nombre del idioma en su propia lengua (`$meta.name`).
    pub fn name(&self) -> &str {
        self.strings.get("$meta.name").map(String::as_str).unwrap_or(&self.code)
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.strings.get(key).map(String::as_str)
    }

    pub fn list(&self, key: &str) -> &[String] {
        self.lists.get(key).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.strings.keys().chain(self.lists.keys()).map(String::as_str)
    }
}

fn flatten(value: &serde_json::Value, prefix: &str, out: &mut Dict) -> Result<(), String> {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                flatten(v, &key, out)?;
            }
            Ok(())
        }
        serde_json::Value::String(s) => {
            out.strings.insert(prefix.to_string(), s.clone());
            Ok(())
        }
        serde_json::Value::Array(items) => {
            let words = items
                .iter()
                .map(|i| i.as_str().map(str::to_string).ok_or_else(|| format!("{prefix}: la lista no es de textos")))
                .collect::<Result<Vec<_>, _>>()?;
            out.lists.insert(prefix.to_string(), words);
            Ok(())
        }
        other => Err(format!("{prefix}: valor no admitido {other}")),
    }
}

fn parse_language(code: &str) -> Result<Dict, String> {
    let (_, json) = LANGUAGES.iter().find(|(c, _)| *c == code).ok_or_else(|| format!("idioma desconocido: {code}"))?;
    Dict::parse(json)
}

static FALLBACK: LazyLock<Dict> = LazyLock::new(|| parse_language(DEFAULT).expect("en.json válido"));
static ACTIVE: RwLock<Option<Dict>> = RwLock::new(None);

/// Activa un idioma de `LANGUAGES` para toda la aplicación. Lo llama la shell al arrancar.
pub fn set_language(code: &str) -> Result<(), String> {
    let dict = parse_language(code)?;
    *ACTIVE.write().expect("diccionario") = Some(dict);
    Ok(())
}

/// Código del idioma activo (`en` si no se activó ninguno).
pub fn language() -> String {
    ACTIVE.read().expect("diccionario").as_ref().map(|d| d.code.clone()).unwrap_or_else(|| DEFAULT.to_string())
}

/// ¿Está este código entre los idiomas incrustados?
pub fn is_available(code: &str) -> bool {
    LANGUAGES.iter().any(|(c, _)| *c == code)
}

/// (código, nombre en su propia lengua) de cada idioma, en el orden de `LANGUAGES`.
pub fn language_names() -> Vec<(String, String)> {
    LANGUAGES
        .iter()
        .filter_map(|(code, json)| Dict::parse(json).ok().map(|d| (code.to_string(), d.name().to_string())))
        .collect()
}

fn with_active<R>(f: impl FnOnce(&Dict) -> R) -> R {
    let guard = ACTIVE.read().expect("diccionario");
    f(guard.as_ref().unwrap_or(&FALLBACK))
}

/// El texto de `key` en el idioma activo, o en inglés, o `[key]`.
pub fn t(key: &str) -> String {
    with_active(|d| lookup(d, key))
}

fn lookup(d: &Dict, key: &str) -> String {
    d.get(key).or_else(|| FALLBACK.get(key)).map(str::to_string).unwrap_or_else(|| format!("[{key}]"))
}

/// Como `t`, sustituyendo cada `{nombre}` por su valor. Los marcadores sin valor se quedan.
pub fn tr(key: &str, args: &[(&str, &dyn Display)]) -> String {
    fill(&t(key), args)
}

/// Singular o plural: `key_one` si `n == 1`, `key_other` si no, con `{n}` sustituido.
pub fn tn(key: &str, n: usize) -> String {
    let full = format!("{key}_{}", if n == 1 { "one" } else { "other" });
    fill(&t(&full), &[("n", &n)])
}

/// Como `tn`, con más marcadores además de `{n}`.
pub fn tnr(key: &str, n: usize, args: &[(&str, &dyn Display)]) -> String {
    fill(&tn(key, n), args)
}

/// Las palabras de `key` en todos los idiomas, sin repetidas (para listas que miran el
/// contenido de la página, no la interfaz).
pub fn all_lists(key: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (_, json) in LANGUAGES {
        if let Ok(d) = Dict::parse(json) {
            for w in d.list(key) {
                if !out.contains(w) {
                    out.push(w.clone());
                }
            }
        }
    }
    out
}

/// Separador decimal del idioma activo (`,` en español, `.` en inglés).
pub fn decimal_separator() -> String {
    t("$meta.decimal_separator")
}

/// Sustituye `{nombre}` por su valor; lo que no tenga valor se deja tal cual.
pub fn fill(text: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = text.to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), &value.to_string());
    }
    out
}

/// Diccionario plano del idioma activo con el inglés debajo, como objeto JSON, para los
/// webviews. Lleva además `$languages`: los idiomas disponibles como `{code, name}`.
pub fn flat_json() -> String {
    with_active(|d| {
        let mut map = serde_json::Map::new();
        for (k, v) in &FALLBACK.strings {
            map.insert(k.clone(), serde_json::Value::String(v.clone()));
        }
        for (k, v) in &FALLBACK.lists {
            map.insert(k.clone(), serde_json::json!(v));
        }
        for (k, v) in &d.strings {
            map.insert(k.clone(), serde_json::Value::String(v.clone()));
        }
        for (k, v) in &d.lists {
            map.insert(k.clone(), serde_json::json!(v));
        }
        let langs: Vec<_> = language_names()
            .into_iter()
            .map(|(code, name)| serde_json::json!({ "code": code, "name": name }))
            .collect();
        map.insert("$languages".into(), serde_json::Value::Array(langs));
        serde_json::Value::Object(map).to_string()
    })
}

/// Código de idioma a partir de un LANGID de Windows (`GetUserDefaultUILanguage`): solo
/// cuenta el idioma primario (`langid & 0x3ff`), no la región.
pub fn language_from_langid(langid: u16) -> Option<&'static str> {
    let code = match langid & 0x3ff {
        0x09 => "en",
        0x0a => "es",
        0x0c => "fr",
        0x16 => "pt",
        0x07 => "de",
        0x10 => "it",
        0x15 => "pl",
        _ => return None,
    };
    is_available(code).then_some(code)
}

/// Código de idioma a partir de una variable de entorno POSIX (`pt_BR.UTF-8`, `es`, `C`).
pub fn language_from_env(value: &str) -> Option<&'static str> {
    let primary = value.split(['_', '-', '.', '@']).next()?.to_ascii_lowercase();
    LANGUAGES.iter().map(|(c, _)| *c).find(|c| *c == primary)
}

/// Qué idioma arranca: el primero de `--lang`, `TEMIKTLI_LANG`, `config.json` y el del
/// sistema que esté entre los incrustados; si ninguno, inglés. `"system"` en la
/// configuración significa «el del sistema».
pub fn resolve_language(cli: Option<&str>, env: Option<&str>, config: Option<&str>, system: Option<&str>) -> String {
    let explicit = [cli, env, config].into_iter().flatten().map(str::trim).find(|v| !v.is_empty());
    match explicit {
        Some("system") | None => {}
        Some(code) if is_available(code) => return code.to_string(),
        Some(_) => {}
    }
    match system {
        Some(code) if is_available(code) => code.to_string(),
        _ => DEFAULT.to_string(),
    }
}

/// Idioma en que el usuario ve el sistema operativo, si está entre los incrustados.
pub fn system_language() -> Option<String> {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetUserDefaultUILanguage() -> u16;
        }
        // Sin argumentos ni memoria compartida: solo devuelve un número.
        let langid = unsafe { GetUserDefaultUILanguage() };
        return language_from_langid(langid).map(str::to_string);
    }
    #[cfg(not(windows))]
    {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .find_map(|v| language_from_env(&v))
            .map(str::to_string)
    }
}

/// `t!("status.saved", path = p.display())` → `tr("status.saved", &[("path", &p.display())])`.
#[macro_export]
macro_rules! t {
    ($key:expr) => {
        $crate::i18n::t($key)
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::tr($key, &[$((stringify!($name), &$value)),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn placeholders(s: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut rest = s;
        while let Some(start) = rest.find('{') {
            let Some(len) = rest[start..].find('}') else { break };
            let name = &rest[start + 1..start + len];
            if !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
                out.insert(name.to_string());
            }
            rest = &rest[start + len + 1..];
        }
        out
    }

    #[test]
    fn every_language_has_the_same_keys_placeholders_and_plurals_as_english() {
        let en = Dict::parse(LANGUAGES[0].1).unwrap();
        let en_keys: BTreeSet<&str> = en.keys().collect();
        for (code, json) in LANGUAGES {
            let d = Dict::parse(json).unwrap_or_else(|e| panic!("{code}: {e}"));
            assert_eq!(d.code(), *code);
            let keys: BTreeSet<&str> = d.keys().collect();
            let missing: Vec<_> = en_keys.difference(&keys).collect();
            let extra: Vec<_> = keys.difference(&en_keys).collect();
            assert!(missing.is_empty() && extra.is_empty(), "{code}: faltan {missing:?}, sobran {extra:?}");
            for key in &en_keys {
                if key.starts_with('$') || key.ends_with("$note") {
                    continue;
                }
                if let (Some(a), Some(b)) = (en.get(key), d.get(key)) {
                    assert_eq!(placeholders(a), placeholders(b), "{code}: marcadores de {key}");
                }
                if let Some(base) = key.strip_suffix("_one") {
                    assert!(d.get(&format!("{base}_other")).is_some(), "{code}: {key} sin _other");
                }
            }
        }
    }

    #[test]
    fn looks_up_with_fallback_and_marks_missing_keys() {
        let es = Dict::parse(LANGUAGES[1].1).unwrap();
        assert_eq!(lookup(&es, "menu.file.open"), "Abrir…");
        assert_eq!(lookup(&es, "no.such.key"), "[no.such.key]");
        // Sin idioma activado, `t` responde en inglés.
        assert_eq!(t("menu.file.open"), "Open…");
        assert_eq!(language(), "en");
    }

    #[test]
    fn fills_placeholders_and_picks_plural_forms() {
        assert_eq!(fill("Saved: {path}", &[("path", &"a.html")]), "Saved: a.html");
        assert_eq!(fill("{a} and {b}", &[("a", &1)]), "1 and {b}");
        assert_eq!(tr("status.saved", &[("path", &"x")]), "Saved: x");
        assert_eq!(tn("units.file", 1), "1 file");
        assert_eq!(tn("units.file", 3), "3 files");
        assert_eq!(t!("status.renamed", from = "a", to = "b"), "Renamed: a → b");
    }

    #[test]
    fn word_lists_are_the_union_of_all_languages() {
        let words = all_lists("lint.vague_link_words");
        for w in ["aquí", "click here", "hier", "ici", "tutaj"] {
            assert!(words.contains(&w.to_string()), "{w}");
        }
        assert_eq!(words.iter().filter(|w| *w == "link").count(), 1, "sin repetidas");
    }

    #[test]
    fn detects_the_language_from_windows_and_posix() {
        assert_eq!(language_from_langid(0x0c0a), Some("es")); // español (España)
        assert_eq!(language_from_langid(0x080a), Some("es")); // español (México)
        assert_eq!(language_from_langid(0x0416), Some("pt")); // portugués (Brasil)
        assert_eq!(language_from_langid(0x0409), Some("en"));
        assert_eq!(language_from_langid(0x0407), Some("de"));
        assert_eq!(language_from_langid(0x0410), Some("it"));
        assert_eq!(language_from_langid(0x0415), Some("pl"));
        assert_eq!(language_from_langid(0x040c), Some("fr"));
        assert_eq!(language_from_langid(0x0411), None); // japonés
        assert_eq!(language_from_env("pt_BR.UTF-8"), Some("pt"));
        assert_eq!(language_from_env("es"), Some("es"));
        assert_eq!(language_from_env("de-AT"), Some("de"));
        assert_eq!(language_from_env("C"), None);
        assert_eq!(language_from_env(""), None);
    }

    #[test]
    fn resolves_the_language_in_order_of_precedence() {
        assert_eq!(resolve_language(Some("fr"), Some("es"), Some("de"), Some("it")), "fr");
        assert_eq!(resolve_language(None, Some("es"), Some("de"), Some("it")), "es");
        assert_eq!(resolve_language(None, None, Some("de"), Some("it")), "de");
        assert_eq!(resolve_language(None, None, None, Some("it")), "it");
        assert_eq!(resolve_language(None, None, None, None), "en");
        // «system» y los valores desconocidos ceden el paso al sistema; sin sistema, inglés.
        assert_eq!(resolve_language(None, None, Some("system"), Some("pl")), "pl");
        assert_eq!(resolve_language(Some("xx"), None, None, Some("pl")), "pl");
        assert_eq!(resolve_language(Some("xx"), None, None, Some("ja")), "en");
        assert_eq!(resolve_language(None, None, Some("system"), None), "en");
    }

    #[test]
    fn flat_json_carries_every_key_and_the_language_list() {
        let json: serde_json::Value = serde_json::from_str(&flat_json()).unwrap();
        assert_eq!(json["menu.file.open"], "Open…");
        assert_eq!(json["$meta.language"], "en");
        assert!(json["lint.vague_link_words"].is_array());
        let langs = json["$languages"].as_array().unwrap();
        assert_eq!(langs.len(), LANGUAGES.len());
        assert_eq!(langs[1]["code"], "es");
        assert_eq!(langs[1]["name"], "Español");
    }

    /// Todos los fuentes y assets que llevan textos de interfaz (sin sus tests).
    const SOURCES: &[(&str, &str)] = &[
        ("main.rs", include_str!("../../temiktli-shell/src/main.rs")),
        ("site.rs", include_str!("../../temiktli-shell/src/site.rs")),
        ("fileops.rs", include_str!("../../temiktli-shell/src/fileops.rs")),
        ("snippets.rs", include_str!("../../temiktli-shell/src/snippets.rs")),
        ("lint.rs", include_str!("lint.rs")),
        ("tags.rs", include_str!("tags.rs")),
        ("dom.rs", include_str!("dom.rs")),
        ("source.rs", include_str!("source.rs")),
        ("command.rs", include_str!("command.rs")),
        ("clean.rs", include_str!("clean.rs")),
        ("toolbar.html", include_str!("../../temiktli-shell/assets/toolbar.html")),
        ("popup.html", include_str!("../../temiktli-shell/assets/popup.html")),
        ("panel.html", include_str!("../../temiktli-shell/assets/panel.html")),
        ("code.html", include_str!("../../temiktli-shell/assets/code.html")),
        ("editor-inject.js", include_str!("../../temiktli-shell/assets/editor-inject.js")),
    ];

    fn without_tests(src: &str) -> &str {
        src.split("#[cfg(test)]").next().unwrap_or(src)
    }

    #[test]
    fn every_key_used_in_the_code_exists_in_english() {
        let en = Dict::parse(LANGUAGES[0].1).unwrap();
        let mut known: BTreeSet<&str> = en.keys().collect();
        // Claves que se arman con un trozo variable (etiqueta, atributo, contenido por defecto…).
        let dynamic = ["lint.deprecated_elements.", "lint.deprecated_attrs.", "content.defaults.", "content.snippets.", "content."];
        known.insert("");
        let mut missing = Vec::new();
        for (file, src) in SOURCES {
            let src = without_tests(src);
            let mut rest = src;
            // Cualquier literal con forma de clave: letras minúsculas y puntos, empezando por un bloque conocido.
            let blocks = ["app", "keys", "toolbar", "menu", "panel", "code", "canvas", "units", "dialog", "status", "snippets", "site", "command", "nesting", "source", "clean", "lint", "content"];
            while let Some(i) = rest.find('"') {
                rest = &rest[i + 1..];
                let Some(j) = rest.find('"') else { break };
                let lit = &rest[..j];
                rest = &rest[j + 1..];
                let looks_like_key = lit.contains('.')
                    && lit.bytes().all(|b| b.is_ascii_lowercase() || b == b'.' || b == b'_' || b.is_ascii_digit())
                    && blocks.iter().any(|b| lit.starts_with(&format!("{b}.")));
                if !looks_like_key || dynamic.iter().any(|d| lit.starts_with(d) && lit.ends_with('.')) {
                    continue;
                }
                let exists = known.contains(lit) || known.contains(format!("{lit}_one").as_str());
                // En los HTML/JS la misma forma aparece en data-i18n="…" y en t('…'), con otras comillas.
                if !exists && !file.ends_with(".html") && !file.ends_with(".js") && lit != "snippets.json" {
                    missing.push(format!("{file}: {lit}"));
                }
            }
        }
        assert!(missing.is_empty(), "claves sin definir en en.json: {missing:?}");
    }

    #[test]
    fn webview_keys_exist_in_english() {
        let en = Dict::parse(LANGUAGES[0].1).unwrap();
        let known: BTreeSet<&str> = en.keys().collect();
        let mut missing = Vec::new();
        for (file, src) in SOURCES.iter().filter(|(f, _)| f.ends_with(".html") || f.ends_with(".js")) {
            for quote in ['"', '\''] {
                let mut rest = *src;
                while let Some(i) = rest.find(quote) {
                    rest = &rest[i + 1..];
                    let Some(j) = rest.find(quote) else { break };
                    let lit = &rest[..j];
                    rest = &rest[j + 1..];
                    let shaped = lit.contains('.')
                        && !lit.contains(' ')
                        && lit.bytes().all(|b| b.is_ascii_lowercase() || b == b'.' || b == b'_' || b.is_ascii_digit())
                        && ["keys.", "toolbar.", "menu.", "panel.", "code.", "canvas.", "units.", "status."].iter().any(|b| lit.starts_with(b));
                    if shaped && !lit.ends_with('.') && !known.contains(lit) && !known.contains(format!("{lit}_one").as_str()) {
                        missing.push(format!("{file}: {lit}"));
                    }
                }
            }
        }
        assert!(missing.is_empty(), "claves de los webviews sin definir: {missing:?}");
    }

    #[test]
    fn no_spanish_interface_text_is_left_in_the_sources() {
        // Detector de olvidos: un literal con tilde o ñ dentro de comillas, fuera de los tests.
        // Los mensajes para quien desarrolla (eprintln!, expect) se admiten.
        let mut found = Vec::new();
        for (file, src) in SOURCES {
            for (n, line) in without_tests(src).lines().enumerate() {
                let t = line.trim_start();
                if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') || t.starts_with("<!--") {
                    continue;
                }
                if line.contains("eprintln!") || line.contains(".expect(") {
                    continue;
                }
                let code = line.split("//").next().unwrap_or(line);
                if code.contains('"') && code.chars().any(|c| "áéíóúñÁÉÍÓÚÑ¿¡«".contains(c)) {
                    found.push(format!("{file}:{}: {}", n + 1, line.trim()));
                }
            }
        }
        assert!(found.is_empty(), "texto en español sin traducir:
{}", found.join("
"));
    }
}
