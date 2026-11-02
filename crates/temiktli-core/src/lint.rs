//! Revisión del documento: validación básica de HTML y accesibilidad. Trabaja sobre el
//! DOM (ya corregido por el parser), así que no detecta errores que el parser repara en
//! silencio, como un `<div>` dentro de un `<p>`. Cada aviso explica el porqué, pensado
//! para quien aprende. El contraste de color no está aquí: necesita los estilos reales
//! de la página y lo calcula el lienzo.

use crate::dom::Document;
use crate::i18n;
use crate::protocol::NodeId;
use crate::{t, tn};
use markup5ever_rcdom::{Handle, NodeData};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Rompe algo o impide usar la página con ayudas técnicas.
    Error,
    /// Mala práctica o elemento obsoleto.
    Warning,
    /// Sugerencia.
    Info,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Validation,
    Accessibility,
}

/// Qué hay que hacer para arreglar un aviso. Solo hay arreglo cuando es seguro y mecánico:
/// si hace falta una decisión (qué describe una imagen, a dónde lleva un enlace) no lo hay.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type")]
pub enum FixAction {
    /// Pone (o quita, con `None`) un atributo de un elemento.
    SetAttr { id: NodeId, name: String, value: Option<String> },
    /// Añade HTML al principio del `<head>`.
    InsertInHead { html: String },
    /// Añade `<!DOCTYPE html>` al principio del archivo.
    AddDoctype,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Fix {
    /// Lo que dice el botón.
    pub label: String,
    pub action: FixAction,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Issue {
    /// Elemento al que se refiere; `None` si es del documento entero.
    pub id: Option<NodeId>,
    pub severity: Severity,
    pub category: Category,
    /// Identificador estable de la regla (`img-alt`, `lang`…).
    pub rule: String,
    pub message: String,
    /// Cómo arreglarlo con un clic, si se puede.
    pub fix: Option<Fix>,
}

/// Cuántos avisos hay de cada gravedad.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReviewCounts {
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
}

impl ReviewCounts {
    pub fn of(issues: &[Issue]) -> Self {
        let mut c = Self::default();
        for i in issues {
            match i.severity {
                Severity::Error => c.errors += 1,
                Severity::Warning => c.warnings += 1,
                Severity::Info => c.infos += 1,
            }
        }
        c
    }

    pub fn total(&self) -> usize {
        self.errors + self.warnings + self.infos
    }

    /// «2 errores, 1 aviso», o «sin problemas» si no hay nada. Para la barra de estado.
    pub fn describe(&self) -> String {
        let parts: Vec<String> = [(self.errors, "units.error"), (self.warnings, "units.warning"), (self.infos, "units.info")]
            .into_iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, key)| tn(key, n))
            .collect();
        if parts.is_empty() { t("lint.summary_ok") } else { parts.join(&t("units.list_comma")) }
    }
}

/// Máximo de avisos devueltos: más que eso no se puede leer y solo estorba.
const MAX_ISSUES: usize = 500;

/// Elementos obsoletos; qué usar en su lugar está en `lint.deprecated_elements.<tag>`.
const DEPRECATED_ELEMENTS: &[&str] =
    &["font", "center", "marquee", "blink", "big", "strike", "tt", "acronym", "frame", "frameset", "basefont", "dir", "applet"];

/// (atributo, elementos donde está obsoleto —vacío = en cualquiera—); qué usar está en
/// `lint.deprecated_attrs.<atributo>`.
const DEPRECATED_ATTRS: &[(&str, &[&str])] = &[
    ("align", &[]),
    ("bgcolor", &[]),
    ("valign", &[]),
    ("cellpadding", &[]),
    ("cellspacing", &[]),
    ("background", &["body", "table", "td", "th"]),
    ("border", &["table", "img"]),
    ("hspace", &[]),
    ("vspace", &[]),
];

struct El {
    id: NodeId,
    tag: String,
    parent: Option<NodeId>,
    attrs: Vec<(String, String)>,
    /// Texto accesible del contenido (incluye el `alt` de imágenes y los `aria-label`).
    name: String,
}

impl El {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn has(&self, name: &str) -> bool {
        self.attr(name).is_some()
    }

    fn non_empty(&self, name: &str) -> bool {
        self.attr(name).is_some_and(|v| !v.trim().is_empty())
    }

    /// ¿Tiene nombre accesible (aria-label, aria-labelledby, title o contenido)?
    fn is_named(&self) -> bool {
        self.non_empty("aria-label")
            || self.non_empty("aria-labelledby")
            || self.non_empty("title")
            || !self.name.trim().is_empty()
    }
}

/// Texto que un lector de pantalla leería para este contenido.
fn content_name(h: &Handle, out: &mut String) {
    match &h.data {
        NodeData::Text { contents } => out.push_str(&contents.borrow()),
        NodeData::Element { name, attrs, .. } => {
            let tag = &*name.local;
            if matches!(tag, "script" | "style" | "template") {
                return;
            }
            let get = |n: &str| {
                attrs.borrow().iter().find(|a| &*a.name.local == n).map(|a| a.value.to_string())
            };
            if let Some(l) = get("aria-label").filter(|l| !l.trim().is_empty()) {
                out.push_str(&l);
                return;
            }
            if tag == "img" {
                if let Some(alt) = get("alt") {
                    out.push_str(&alt);
                }
                return;
            }
            for c in h.children.borrow().iter() {
                content_name(c, out);
            }
        }
        _ => {}
    }
}

impl Document {
    /// Revisa el documento y devuelve los avisos, los más graves primero y, dentro de cada
    /// gravedad, en el orden del documento.
    pub fn lint(&self) -> Vec<Issue> {
        let els: Vec<El> = self
            .element_tree()
            .into_iter()
            .filter_map(|(id, parent)| {
                let node = self.nodes.get(&id)?;
                let mut name = String::new();
                content_name(node, &mut name);
                Some(El { id, tag: self.tag(id)?, parent, attrs: self.attrs(id), name })
            })
            .collect();
        let index: HashMap<NodeId, usize> = els.iter().enumerate().map(|(i, e)| (e.id, i)).collect();
        let parent_el = |e: &El| e.parent.and_then(|p| index.get(&p)).map(|&i| &els[i]);

        let mut out = Vec::new();
        let mut push = |id: Option<NodeId>, severity, category, rule: &str, message: String| {
            out.push(Issue { id, severity, category, rule: rule.to_string(), message, fix: None });
        };
        use Category::{Accessibility as A, Validation as V};
        use Severity::{Error as E, Info as I, Warning as W};

        let by_tag = |t: &str| els.iter().find(|e| e.tag == t);
        let body_has_content = by_tag("body").is_some_and(|b| !b.name.trim().is_empty());

        // ----- Documento -----
        let has_doctype = self
            .dom
            .document
            .children
            .borrow()
            .iter()
            .any(|c| matches!(c.data, NodeData::Doctype { .. }));
        if !has_doctype {
            push(
                by_tag("html").map(|e| e.id),
                W,
                V,
                "doctype",
                t("lint.doctype"),
            );
        }
        if let Some(html) = by_tag("html")
            && !html.non_empty("lang")
        {
            push(
                Some(html.id),
                E,
                A,
                "lang",
                t("lint.lang"),
            );
        }
        let head = by_tag("head").map(|e| e.id);
        match els.iter().find(|e| e.tag == "title" && parent_el(e).is_some_and(|p| p.tag == "head")) {
            None => push(head, W, V, "title", t("lint.title_missing")),
            Some(title) if title.name.trim().is_empty() => {
                push(Some(title.id), W, V, "title", t("lint.title_empty"));
            }
            Some(_) => {}
        }
        let has_charset = els.iter().any(|e| {
            e.tag == "meta"
                && (e.has("charset")
                    || e.attr("http-equiv").is_some_and(|v| v.eq_ignore_ascii_case("content-type")))
        });
        if !has_charset {
            push(head, W, V, "charset", t("lint.charset"));
        }
        if !els.iter().any(|e| e.tag == "meta" && e.attr("name").is_some_and(|n| n.eq_ignore_ascii_case("viewport"))) {
            push(head, I, V, "viewport", t("lint.viewport"));
        }

        // ----- Identificadores repetidos -----
        let mut seen: HashMap<&str, NodeId> = HashMap::new();
        for e in &els {
            if let Some(v) = e.attr("id").filter(|v| !v.is_empty())
                && seen.insert(v, e.id).is_some()
            {
                push(Some(e.id), E, V, "duplicate-id", t!("lint.duplicate_id", id = v));
            }
        }

        // ----- Encabezados -----
        let headings: Vec<(&El, u8)> = els
            .iter()
            .filter_map(|e| {
                let b = e.tag.as_bytes();
                (b.len() == 2 && b[0] == b'h' && (b'1'..=b'6').contains(&b[1])).then(|| (e, b[1] - b'0'))
            })
            .collect();
        let h1s: Vec<&El> = headings.iter().filter(|(_, l)| *l == 1).map(|(e, _)| *e).collect();
        if h1s.is_empty() && body_has_content {
            push(by_tag("body").map(|e| e.id), I, A, "h1-missing", t("lint.h1_missing"));
        }
        for e in h1s.iter().skip(1) {
            push(Some(e.id), W, A, "h1-multiple", t!("lint.h1_multiple", n = h1s.len()));
        }
        let mut prev = 0u8;
        for (e, level) in &headings {
            if prev != 0 && *level > prev + 1 {
                push(Some(e.id), W, A, "heading-skip", t!("lint.heading_skip", prev = prev, level = level));
            }
            if e.name.trim().is_empty() {
                push(Some(e.id), E, A, "heading-empty", t!("lint.heading_empty", level = level));
            }
            prev = *level;
        }
        if by_tag("main").is_none() && body_has_content {
            push(by_tag("body").map(|e| e.id), I, A, "main-missing", t("lint.main_missing"));
        }

        // ----- Etiquetas de formulario -----
        let labelled_ids: HashSet<&str> =
            els.iter().filter(|e| e.tag == "label").filter_map(|e| e.attr("for")).collect();
        let has_ancestor = |e: &El, tag: &str| {
            let mut cur = parent_el(e);
            while let Some(p) = cur {
                if p.tag == tag {
                    return true;
                }
                cur = parent_el(p);
            }
            false
        };
        let tables_with_th: HashSet<NodeId> = els
            .iter()
            .filter(|e| e.tag == "th")
            .flat_map(|th| {
                let mut tables = Vec::new();
                let mut cur = parent_el(th);
                while let Some(p) = cur {
                    if p.tag == "table" {
                        tables.push(p.id);
                    }
                    cur = parent_el(p);
                }
                tables
            })
            .collect();

        // ----- Elemento por elemento -----
        // Las palabras que delatan un alt o un enlace vagos se miran en todos los idiomas: el
        // contenido de la página no tiene por qué estar en el de la interfaz.
        let poor_alt = i18n::all_lists("lint.poor_alt_words");
        let vague_link = i18n::all_lists("lint.vague_link_words");
        for e in &els {
            let tag = e.tag.as_str();
            if DEPRECATED_ELEMENTS.contains(&tag) {
                let hint = t(&format!("lint.deprecated_elements.{tag}"));
                push(Some(e.id), W, V, "deprecated-element", t!("lint.deprecated_element", tag = tag, hint = hint));
            }
            for (attr, only) in DEPRECATED_ATTRS {
                if e.has(attr) && (only.is_empty() || only.contains(&tag)) {
                    let hint = t(&format!("lint.deprecated_attrs.{attr}"));
                    push(Some(e.id), W, V, "deprecated-attr", t!("lint.deprecated_attr", attr = attr, tag = tag, hint = hint));
                }
            }
            for (k, _) in &e.attrs {
                if k.len() > 2 && k.starts_with("on") && k.bytes().all(|b| b.is_ascii_lowercase()) {
                    push(Some(e.id), I, V, "inline-handler", t!("lint.inline_handler", attr = k));
                }
            }
            if let Some(v) = e.attr("tabindex").and_then(|v| v.trim().parse::<i32>().ok())
                && v > 0
            {
                push(Some(e.id), W, A, "tabindex-positive", t!("lint.tabindex_positive", value = v));
            }

            match tag {
                "img" => match e.attr("alt") {
                    None => push(Some(e.id), E, A, "img-alt", t("lint.img_alt")),
                    Some(alt) => {
                        let low = alt.trim().to_lowercase();
                        let looks_like_file = [".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp"]
                            .iter()
                            .any(|x| low.ends_with(x));
                        if looks_like_file || poor_alt.iter().any(|w| *w == low) {
                            push(Some(e.id), W, A, "img-alt-poor", t!("lint.img_alt_poor", alt = alt));
                        }
                    }
                },
                "input" if e.attr("type").is_some_and(|ty| ty.eq_ignore_ascii_case("image")) && !e.non_empty("alt") => {
                    push(Some(e.id), E, A, "img-alt", t("lint.img_alt_button"));
                }
                "a" if e.has("href") => {
                    if !e.is_named() {
                        push(Some(e.id), E, A, "link-name", t("lint.link_name"));
                    } else {
                        let text = e.name.trim().trim_matches(|c: char| c.is_ascii_punctuation()).to_lowercase();
                        if vague_link.iter().any(|w| *w == text) {
                            push(Some(e.id), W, A, "link-vague", t!("lint.link_vague", text = e.name.trim()));
                        }
                    }
                    if e.attr("target") == Some("_blank") && !e.attr("rel").is_some_and(|r| r.contains("noopener") || r.contains("noreferrer")) {
                        push(Some(e.id), I, V, "target-blank", t("lint.target_blank"));
                    }
                    if e.attr("href").is_some_and(|h| h.trim() == "#" || h.trim().is_empty()) {
                        push(Some(e.id), I, V, "link-placeholder", t("lint.link_placeholder"));
                    }
                }
                "button" if !e.is_named() => {
                    push(Some(e.id), E, A, "button-name", t("lint.button_name"));
                }
                "input" | "select" | "textarea" => {
                    let ty = e.attr("type").unwrap_or("text").to_ascii_lowercase();
                    let needs_label = tag != "input" || !matches!(ty.as_str(), "hidden" | "submit" | "button" | "reset" | "image");
                    let labelled = e.non_empty("aria-label")
                        || e.non_empty("aria-labelledby")
                        || e.non_empty("title")
                        || e.attr("id").is_some_and(|i| labelled_ids.contains(i))
                        || has_ancestor(e, "label");
                    if needs_label && !labelled {
                        push(Some(e.id), E, A, "form-label", t!("lint.form_label", tag = tag));
                    }
                }
                "iframe" if !e.non_empty("title") => {
                    push(Some(e.id), W, A, "iframe-title", t("lint.iframe_title"));
                }
                "table" if !tables_with_th.contains(&e.id) => {
                    push(Some(e.id), I, A, "table-headers", t("lint.table_headers"));
                }
                _ => {}
            }
        }

        for issue in &mut out {
            issue.fix = self.fix_for(issue);
        }
        out.sort_by_key(|i| (i.severity, i.id.unwrap_or(0)));
        out.truncate(MAX_ISSUES);
        out
    }

    /// El arreglo de un clic para este aviso, si lo hay.
    fn fix_for(&self, issue: &Issue) -> Option<Fix> {
        let set = |id: NodeId, name: &str, value: &str, label: String| Fix {
            label,
            action: FixAction::SetAttr { id, name: name.into(), value: Some(value.into()) },
        };
        let head = |label: String, html: String| Fix { label, action: FixAction::InsertInHead { html } };
        let id = issue.id?;
        let tag = self.tag(id).unwrap_or_default();
        Some(match issue.rule.as_str() {
            // El idioma que se propone es el de las páginas que crea el editor (bloque `content`).
            "lang" => {
                let lang = t("content.document_lang");
                set(id, "lang", &lang, t!("lint.fix.lang", lang = lang))
            }
            // Solo en <img>: un botón de imagen necesita un texto de verdad, no uno vacío.
            "img-alt" if tag == "img" => set(id, "alt", "", t("lint.fix.img_alt_decorative")),
            "charset" => head(t("lint.fix.charset"), "<meta charset=\"utf-8\">".into()),
            "viewport" => head(
                t("lint.fix.viewport"),
                "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">".into(),
            ),
            // Con el <title> vacío el aviso apunta al <title>: ahí no hay nada que añadir.
            "title" if tag == "head" => head(t("lint.fix.title"), format!("<title>{}</title>", t("content.title_placeholder"))),
            "doctype" => Fix { label: t("lint.fix.doctype"), action: FixAction::AddDoctype },
            "target-blank" => {
                let rel = self.attr(id, "rel").unwrap_or_default();
                let value = if rel.trim().is_empty() { "noopener".to_string() } else { format!("{} noopener", rel.trim()) };
                set(id, "rel", &value, t("lint.fix.noopener"))
            }
            "tabindex-positive" => set(id, "tabindex", "0", t("lint.fix.tabindex")),
            "iframe-title" => set(id, "title", &t("content.iframe_title"), t("lint.fix.iframe_title")),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(html: &str) -> Vec<String> {
        Document::parse(html).lint().into_iter().map(|i| i.rule).collect()
    }

    const GOOD: &str = r#"<!DOCTYPE html><html lang="es"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width"><title>Hola</title></head>
<body><main><h1>Título</h1><h2>Sección</h2><p>Texto con <a href="/x">un enlace claro</a>.</p>
<img src="a.png" alt="Un gato dormido"><label for="n">Nombre</label><input id="n"></main></body></html>"#;

    #[test]
    fn a_well_formed_page_has_no_issues() {
        let issues = Document::parse(GOOD).lint();
        assert!(issues.is_empty(), "{issues:#?}");
    }

    #[test]
    fn reports_document_level_problems_on_a_bare_page() {
        let r = rules("<p>hola</p>");
        for rule in ["doctype", "lang", "title", "charset", "viewport", "h1-missing", "main-missing"] {
            assert!(r.contains(&rule.to_string()), "falta {rule}: {r:?}");
        }
        // Un título vacío cuenta como ausente.
        assert!(rules("<!DOCTYPE html><html lang=es><head><title> </title></head><body></body></html>").contains(&"title".to_string()));
    }

    #[test]
    fn accessibility_errors_come_first_and_point_at_the_element() {
        let d = Document::parse(&GOOD.replace("alt=\"Un gato dormido\"", "").replace("<label for=\"n\">Nombre</label>", ""));
        let issues = d.lint();
        assert_eq!(issues[0].severity, Severity::Error);
        let img = issues.iter().find(|i| i.rule == "img-alt").unwrap();
        assert_eq!(d.tag(img.id.unwrap()).as_deref(), Some("img"));
        let field = issues.iter().find(|i| i.rule == "form-label").unwrap();
        assert_eq!(d.tag(field.id.unwrap()).as_deref(), Some("input"));
        // Orden: ninguna advertencia antes de un error.
        let first_non_error = issues.iter().position(|i| i.severity != Severity::Error).unwrap_or(issues.len());
        assert!(issues[first_non_error..].iter().all(|i| i.severity != Severity::Error));
    }

    #[test]
    fn decorative_images_and_wrapped_labels_are_accepted() {
        let html = GOOD.replace("alt=\"Un gato dormido\"", "alt=\"\"").replace(
            "<label for=\"n\">Nombre</label><input id=\"n\">",
            "<label>Nombre <input></label>",
        );
        assert!(Document::parse(&html).lint().is_empty());
    }

    #[test]
    fn link_and_button_names_are_taken_from_their_content() {
        let r = rules(&GOOD.replace("<p>", "<p><a href=\"/y\"></a><a href=\"/z\"><img src=\"i.png\" alt=\"Inicio\"></a><button></button><button>Enviar</button><a href=\"/q\">aquí</a>"));
        assert_eq!(r.iter().filter(|x| *x == "link-name").count(), 1);
        assert_eq!(r.iter().filter(|x| *x == "button-name").count(), 1);
        assert_eq!(r.iter().filter(|x| *x == "link-vague").count(), 1);
    }

    #[test]
    fn finds_duplicate_ids_heading_jumps_and_extra_h1() {
        let r = rules(&GOOD.replace("<h2>Sección</h2>", "<h3>Salto</h3><h1>Otro</h1><div id=\"a\"></div><div id=\"a\"></div>"));
        assert_eq!(r.iter().filter(|x| *x == "duplicate-id").count(), 1);
        assert!(r.contains(&"heading-skip".to_string()));
        assert!(r.contains(&"h1-multiple".to_string()));
    }

    #[test]
    fn flags_deprecated_markup_and_inline_handlers() {
        let r = rules(&GOOD.replace("<p>", "<center onclick=\"x()\" bgcolor=\"red\">hola</center><p>"));
        for rule in ["deprecated-element", "deprecated-attr", "inline-handler"] {
            assert!(r.contains(&rule.to_string()), "falta {rule}: {r:?}");
        }
    }

    #[test]
    fn small_checks_cover_iframes_tabindex_tables_and_blank_targets() {
        let r = rules(&GOOD.replace("<p>", "<iframe src=\"x\"></iframe><div tabindex=\"3\"></div><table><tr><td>1</td></tr></table><a href=\"/w\" target=\"_blank\">Fuera</a><p>"));
        for rule in ["iframe-title", "tabindex-positive", "table-headers", "target-blank"] {
            assert!(r.contains(&rule.to_string()), "falta {rule}: {r:?}");
        }
        // Una tabla con encabezados no avisa.
        assert!(!rules(&GOOD.replace("<p>", "<table><tr><th>A</th></tr></table><p>")).contains(&"table-headers".to_string()));
    }

    /// Aplica un arreglo como lo hace la aplicación y devuelve el documento resultante.
    fn apply(d: &Document, fix: &Fix) -> Document {
        let saved = d.to_saved_html().html;
        match &fix.action {
            FixAction::SetAttr { id, name, value } => {
                let mut d2 = Document::parse(&saved);
                d2.set_attr(*id, name, value.clone());
                d2
            }
            FixAction::InsertInHead { html } => Document::parse(&d.with_head_prepended(&saved, html).unwrap()),
            FixAction::AddDoctype => Document::parse(&d.with_doctype(&saved).unwrap()),
        }
    }

    #[test]
    fn every_one_click_fix_removes_its_issue_and_what_is_left_has_no_fix() {
        let bad = r##"<html><head></head><body><img src="a.png"><a href="/x" target="_blank" rel="nofollow">Ir</a>
<div tabindex="2">x</div><iframe src="y"></iframe></body></html>"##;
        let mut doc = Document::parse(bad);
        let mut applied = Vec::new();
        for _ in 0..30 {
            let issues = doc.lint();
            let Some(issue) = issues.iter().find(|i| i.fix.is_some()) else { break };
            let fix = issue.fix.clone().unwrap();
            let before = issues.iter().filter(|i| i.rule == issue.rule).count();
            doc = apply(&doc, &fix);
            let after = doc.lint().iter().filter(|i| i.rule == issue.rule).count();
            assert!(after < before, "«{}» no quitó el aviso {}", fix.label, issue.rule);
            applied.push(issue.rule.clone());
        }
        applied.sort();
        for rule in ["charset", "doctype", "iframe-title", "img-alt", "lang", "tabindex-positive", "target-blank", "title", "viewport"] {
            assert!(applied.contains(&rule.to_string()), "nadie arregló {rule}: {applied:?}");
        }
        // Lo que queda son avisos que piden una decisión tuya, no un arreglo mecánico.
        assert!(doc.lint().iter().all(|i| i.fix.is_none()), "{:#?}", doc.lint());
        // El rel que ya había se conserva al añadir noopener.
        let saved = doc.to_saved_html().html;
        assert!(saved.contains("rel=\"nofollow noopener\""), "{saved}");
    }

    #[test]
    fn some_issues_get_no_fix_because_they_need_a_decision() {
        let d = Document::parse(&GOOD.replace("alt=\"Un gato dormido\"", "alt=\"foto.png\"").replace("<h2>Sección</h2>", "<h4>Salto</h4>"));
        for issue in d.lint() {
            assert!(issue.fix.is_none(), "{} no debería tener arreglo", issue.rule);
        }
        // Un botón de imagen sin alt necesita un texto de verdad: no se arregla con un alt vacío.
        let d = Document::parse(&GOOD.replace("<main>", "<main><input type=\"image\" src=\"b.png\">"));
        let issue = d.lint().into_iter().find(|i| i.rule == "img-alt").unwrap();
        assert!(issue.fix.is_none());
    }

    #[test]
    fn adding_to_the_head_keeps_the_original_layout() {
        let html = "<!DOCTYPE html>\n<html lang=\"es\">\n<head>\n  <title>T</title>\n</head>\n<body>\n<p>x</p>\n</body>\n</html>";
        let d = Document::parse(html);
        let new = d.with_head_prepended(html, "<meta charset=\"utf-8\">").unwrap();
        assert_eq!(
            new,
            "<!DOCTYPE html>\n<html lang=\"es\">\n<head>\n  <meta charset=\"utf-8\">\n  <title>T</title>\n</head>\n<body>\n<p>x</p>\n</body>\n</html>"
        );
        // En una línea, se queda en esa línea.
        let compact = "<!DOCTYPE html><html><head><title>T</title></head><body></body></html>";
        let d = Document::parse(compact);
        let new = d.with_head_prepended(compact, "<meta charset=\"utf-8\">").unwrap();
        assert_eq!(new, "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>T</title></head><body></body></html>");
        // Sin <head> escrito pero con <html>: se crea el <head>.
        let no_head = "<!DOCTYPE html><html lang=es><body><p>x</p></body></html>";
        let d = Document::parse(no_head);
        let new = d.with_head_prepended(no_head, "<title>Hola</title>").unwrap();
        assert!(new.contains("<html lang=es><head><title>Hola</title></head><body>"), "{new}");
        // Sin <html> ni <head> escritos, no se adivina.
        let bare = "<p>x</p>";
        assert!(Document::parse(bare).with_head_prepended(bare, "<title>Hola</title>").is_err());
        // Lo que rompería el documento se rechaza.
        let d = Document::parse(html);
        assert!(d.with_head_prepended(html, "</head><p>x").is_err());
    }

    #[test]
    fn the_doctype_goes_first_and_the_rest_is_untouched() {
        let html = "<html lang=\"es\"><head></head><body><p>x</p></body></html>";
        let d = Document::parse(html);
        let new = d.with_doctype(html).unwrap();
        assert_eq!(new, format!("<!DOCTYPE html>\n{html}"));
        assert!(!Document::parse(&new).lint().iter().any(|i| i.rule == "doctype"));
    }

    #[test]
    fn counts_are_per_severity_and_described_in_plain_spanish() {
        assert_eq!(ReviewCounts::of(&Document::parse(GOOD).lint()).describe(), "no problems");
        let bad = Document::parse("<p>hola</p><img src=a.png>").lint();
        let c = ReviewCounts::of(&bad);
        assert_eq!(c.total(), bad.len());
        assert!(c.errors >= 2 && c.warnings >= 1 && c.infos >= 1, "{c:?}");
        assert_eq!(c.errors + c.warnings + c.infos, c.total());
        // Singular y plural, y los ceros no salen.
        assert_eq!(ReviewCounts { errors: 1, warnings: 0, infos: 3 }.describe(), "1 error, 3 suggestions");
        assert_eq!(ReviewCounts { errors: 2, warnings: 1, infos: 0 }.describe(), "2 errors, 1 warning");
        assert_eq!(ReviewCounts::default().describe(), "no problems");
    }

    #[test]
    fn is_robust_on_odd_documents() {
        for html in ["", "<", "<html>", "<svg><title>x</title></svg>", "<a href><b></a>", "<input type=hidden>"] {
            let _ = Document::parse(html).lint();
        }
    }
}
