//! Snippets: fragmentos HTML con nombre que se guardan una vez y se reinsertan donde se quiera.
//! Los de serie vienen con el editor y no se tocan; los del usuario se guardan en un archivo
//! suyo (`%APPDATA%\Temiktli\snippets.json`), no dentro de cada sitio, para que estén en
//! cualquier documento y no ensucien las carpetas del proyecto.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use temiktli_core::{i18n, t};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Snippet {
    pub name: String,
    pub html: String,
}

/// Tope de snippets propios y de tamaño de cada uno: lo que pasa de ahí no es un fragmento
/// reutilizable, es un documento.
pub const MAX_SNIPPETS: usize = 200;
pub const MAX_HTML_BYTES: usize = 100_000;
pub const MAX_NAME_CHARS: usize = 60;

/// Imagen gris de 400x200 para que la tarjeta se vea (como la de «Insertar img»).
const PLACEHOLDER: &str = "data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='400' height='200'><rect width='400' height='200' fill='%23ccd'/></svg>";

/// Los snippets que vienen con el editor, para empezar, en el idioma activo (bloque `content`).
pub fn builtins() -> Vec<Snippet> {
    let snippet = |key: &str| Snippet {
        name: t(&format!("content.snippets.{key}.name")),
        html: i18n::fill(&t(&format!("content.snippets.{key}.html")), &[("placeholder", &PLACEHOLDER)]),
    };
    ["card", "header", "contact_form", "footer", "section"].into_iter().map(snippet).collect()
}

/// Dónde se guarda el archivo del usuario: la carpeta de datos de aplicaciones del sistema.
pub fn storage_path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))?;
    Some(base.join("Temiktli").join("snippets.json"))
}

/// Un nombre válido: con texto, sin saltos de línea y no demasiado largo.
pub fn clean_name(raw: &str) -> Result<String, String> {
    let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err(t("snippets.name_empty"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(t!("snippets.name_too_long", max = MAX_NAME_CHARS));
    }
    Ok(name)
}

/// Lo que se lee del archivo. Si está corrupto no se pierde: se aparta como `.bad` y se
/// avisa, para que el siguiente guardado no lo pise con una lista vacía.
pub struct Loaded {
    pub snippets: Vec<Snippet>,
    pub warning: Option<String>,
}

pub fn load(path: &Path) -> Loaded {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Loaded { snippets: Vec::new(), warning: None };
        }
        Err(e) => {
            return Loaded { snippets: Vec::new(), warning: Some(t!("snippets.read_failed", error = e)) };
        }
    };
    match serde_json::from_slice::<Vec<Snippet>>(&bytes) {
        Ok(list) => {
            // Lo que no cumple los límites (archivo editado a mano) se ignora, no se arregla.
            let snippets = list
                .into_iter()
                .filter(|s| clean_name(&s.name).is_ok() && !s.html.trim().is_empty() && s.html.len() <= MAX_HTML_BYTES)
                .take(MAX_SNIPPETS)
                .collect();
            Loaded { snippets, warning: None }
        }
        Err(_) => {
            let mut bad = path.as_os_str().to_owned();
            bad.push(".bad");
            let kept = std::fs::rename(path, &bad).is_ok();
            Loaded {
                snippets: Vec::new(),
                warning: Some(t(if kept { "snippets.file_damaged_moved" } else { "snippets.file_damaged" })),
            }
        }
    }
}

/// Guarda la lista escribiendo primero un archivo temporal y renombrándolo: un corte a mitad
/// no deja el archivo a medias.
pub fn save(path: &Path, snippets: &[Snippet]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(snippets).map_err(|e| e.to_string())?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use temiktli_core::{Document, ReviewCounts, SaveMode};

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tk-snippets-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn names_are_trimmed_squashed_and_bounded() {
        assert_eq!(clean_name("  Mi   tarjeta \n nueva ").unwrap(), "Mi tarjeta nueva");
        assert!(clean_name("   ").is_err());
        assert!(clean_name(&"a".repeat(MAX_NAME_CHARS)).is_ok());
        assert!(clean_name(&"a".repeat(MAX_NAME_CHARS + 1)).is_err());
        // Cuenta letras, no bytes: las tildes no estorban.
        assert!(clean_name(&"ñ".repeat(MAX_NAME_CHARS)).is_ok());
    }

    #[test]
    fn saved_snippets_come_back_exactly_and_a_missing_file_is_just_empty() {
        let d = dir("roundtrip");
        let file = d.join("Temiktli").join("snippets.json"); // la carpeta no existe aún
        assert!(load(&file).snippets.is_empty() && load(&file).warning.is_none());
        let list = vec![
            Snippet { name: "Tarjeta".into(), html: "<div class=\"a\">\n  <p>ñandú \"x\" &amp; y</p>\n</div>".into() },
            Snippet { name: "Otra".into(), html: "<hr>".into() },
        ];
        save(&file, &list).unwrap();
        let back = load(&file);
        assert_eq!(back.snippets, list);
        assert!(back.warning.is_none());
        // No queda el archivo temporal.
        assert!(!file.with_extension("json.tmp").exists());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_damaged_file_is_kept_aside_not_overwritten() {
        let d = dir("corrupt");
        let file = d.join("snippets.json");
        std::fs::write(&file, "{ esto no es una lista").unwrap();
        let loaded = load(&file);
        assert!(loaded.snippets.is_empty());
        assert!(loaded.warning.as_deref().unwrap().contains("snippets.json.bad"));
        assert_eq!(std::fs::read_to_string(d.join("snippets.json.bad")).unwrap(), "{ esto no es una lista");
        assert!(!file.exists(), "el original se apartó, no se dejó para pisarlo");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn entries_that_break_the_limits_are_ignored_on_load() {
        let d = dir("limits");
        let file = d.join("s.json");
        let mut list = vec![
            Snippet { name: "Bien".into(), html: "<p>x</p>".into() },
            Snippet { name: "".into(), html: "<p>sin nombre</p>".into() },
            Snippet { name: "Vacío".into(), html: "  ".into() },
            Snippet { name: "Enorme".into(), html: "x".repeat(MAX_HTML_BYTES + 1) },
        ];
        std::fs::write(&file, serde_json::to_string(&list).unwrap()).unwrap();
        assert_eq!(load(&file).snippets, vec![list.remove(0)]);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn every_built_in_snippet_inserts_cleanly_and_is_good_html() {
        let page = "<!DOCTYPE html><html lang=\"es\"><head><meta charset=\"utf-8\">\
                    <meta name=\"viewport\" content=\"width=device-width\"><title>T</title></head>\
                    <body><main><h2>Hola</h2><p id=\"p\">Texto</p></main></body></html>";
        let builtins = builtins();
        assert_eq!(builtins.len(), 5);
        let mut names = std::collections::HashSet::new();
        for b in &builtins {
            assert!(names.insert(b.name.clone()), "nombre repetido: {}", b.name);
            assert!(clean_name(&b.name).is_ok());
            assert!(b.html.len() < MAX_HTML_BYTES);

            let d = Document::parse(page);
            let saved = d.to_saved_html().html;
            let sel = (1..200).find(|&n| d.attr(n, "id").as_deref() == Some("p")).unwrap();
            let n = temiktli_core::fragment_elements(&b.html);
            assert!(n > 0, "{}", b.name);
            let new = d.with_pasted(&saved, sel, &b.html, n).unwrap_or_else(|e| panic!("{}: {e}", b.name));

            // El resultado se guarda tal cual (sin que el navegador reorganice nada) y no
            // trae errores de HTML ni de accesibilidad: un snippet de serie da buen ejemplo.
            let after = Document::parse(&new);
            assert!(matches!(after.to_saved_html().mode, SaveMode::Preserved), "{} se reformatea", b.name);
            let counts = ReviewCounts::of(&after.lint());
            assert_eq!(counts.errors, 0, "{}: {:#?}", b.name, after.lint());
            assert_eq!(counts.warnings, 0, "{}: {:#?}", b.name, after.lint());
        }
    }
}
