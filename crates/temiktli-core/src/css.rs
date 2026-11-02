//! Lectura y edición de hojas de estilo `<style>`, para el panel de estilos.
//!
//! `cssparser` (MPL-2.0, sin modificar) tokeniza el CSS y da la posición de cada pieza;
//! aquí solo se reconoce la estructura mínima: reglas de estilo (también dentro de
//! `@media`, `@supports`, `@layer` y `@container`) y sus declaraciones. Las ediciones
//! son cambios puntuales sobre el texto, así que el resto de la hoja queda intacto.
//! Qué reglas afectan a un elemento lo decide el navegador del lienzo, no este módulo.

use crate::dom::Document;
use crate::protocol::NodeId;
use cssparser::{ParseError, Parser, Token};
use serde::{Deserialize, Serialize};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub struct CssDecl {
    pub name: String,
    pub value: String,
    pub important: bool,
    /// Desde el nombre hasta el final del valor (con `!important`), sin el `;`.
    pub range: Range<usize>,
    /// Solo el valor, sin `!important` ni espacios en los bordes.
    pub value_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CssRule {
    pub selector: String,
    /// Cabecera del at-rule que la contiene, p. ej. `@media (max-width: 600px)`.
    pub context: Option<String>,
    /// Interior de las llaves.
    pub block: Range<usize>,
    pub decls: Vec<CssDecl>,
}

/// Una hoja de estilo que usa el documento, en el orden en que aparece.
#[derive(Debug, Clone, PartialEq)]
pub enum Sheet {
    /// Un elemento `<style>` con su CSS.
    Inline { style: NodeId, css: String },
    /// Un `<link rel="stylesheet" href="…">`: su contenido está en otro archivo.
    Linked { href: String },
}

/// Regla tal como la ve el panel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuleView {
    /// Archivo `.css` externo que la contiene (ruta relativa); `None` si está en un `<style>`.
    #[serde(default)]
    pub file: Option<String>,
    /// Elemento `<style>` que la contiene (0 si es de un archivo externo).
    pub style: NodeId,
    /// Posición de la regla dentro de esa hoja.
    pub index: usize,
    pub selector: String,
    pub context: Option<String>,
    pub decls: Vec<DeclView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeclView {
    pub name: String,
    pub value: String,
    pub important: bool,
}

const NESTING_AT_RULES: &[&str] = &["media", "supports", "layer", "container"];

pub fn parse_rules(css: &str) -> Vec<CssRule> {
    let mut parser = Parser::new(css);
    let mut out = Vec::new();
    read_rules(css, &mut parser, None, &mut out);
    out
}

fn read_rules(css: &str, p: &mut Parser<'_>, ctx: Option<&str>, out: &mut Vec<CssRule>) {
    let mut prelude: Option<usize> = None;
    loop {
        let before = p.position().byte_index();
        let tok = match p.next_including_whitespace_and_comments() {
            Ok(t) => t.clone(),
            Err(_) => break,
        };
        let after = p.position().byte_index();
        match tok {
            Token::WhiteSpace(_) | Token::Comment(_) if prelude.is_none() => {}
            Token::Semicolon => prelude = None,
            Token::CurlyBracketBlock => {
                let start = prelude.take().unwrap_or(before);
                let head = css[start..after - 1].trim();
                if let Some(at) = head.strip_prefix('@') {
                    let name = at.split(|c: char| !c.is_ascii_alphanumeric() && c != '-').next().unwrap_or("");
                    if NESTING_AT_RULES.contains(&name.to_ascii_lowercase().as_str()) {
                        let _ = p.parse_nested_block(|inner| {
                            read_rules(css, inner, Some(head), out);
                            Ok::<(), ParseError<()>>(())
                        });
                    } else {
                        skip_block(p);
                    }
                } else {
                    let decls = p
                        .parse_nested_block(|inner| Ok::<_, ParseError<()>>(read_decls(css, inner)))
                        .unwrap_or_default();
                    let end = p.position().byte_index();
                    let closed = end > after && css.as_bytes()[end - 1] == b'}';
                    out.push(CssRule {
                        selector: head.to_string(),
                        context: ctx.map(str::to_string),
                        block: after..if closed { end - 1 } else { end },
                        decls,
                    });
                }
            }
            _ => {
                if prelude.is_none() {
                    prelude = Some(before);
                }
            }
        }
    }
}

fn skip_block(p: &mut Parser<'_>) {
    let _ = p.parse_nested_block(|inner| {
        while inner.next_including_whitespace_and_comments().is_ok() {}
        Ok::<(), ParseError<()>>(())
    });
}

fn read_decls(css: &str, p: &mut Parser<'_>) -> Vec<CssDecl> {
    let mut decls = Vec::new();
    let mut start: Option<usize> = None;
    loop {
        let before = p.position().byte_index();
        let tok = match p.next_including_whitespace_and_comments() {
            Ok(t) => t.clone(),
            Err(_) => break,
        };
        match tok {
            Token::WhiteSpace(_) | Token::Comment(_) if start.is_none() => {}
            Token::Semicolon => {
                // El `;` mide un byte; `before` no es fiable tras un bloque saltado.
                let end = p.position().byte_index() - 1;
                if let Some(s) = start.take() {
                    push_decl(css, s, end, &mut decls);
                }
            }
            // Regla anidada (CSS nesting): no es una declaración.
            Token::CurlyBracketBlock => {
                start = None;
                skip_block(p);
            }
            _ => {
                if start.is_none() {
                    start = Some(before);
                }
            }
        }
    }
    if let Some(s) = start {
        push_decl(css, s, p.position().byte_index(), &mut decls);
    }
    decls
}

fn push_decl(css: &str, start: usize, end: usize, out: &mut Vec<CssDecl>) {
    let Some(text) = css.get(start..end) else { return };
    let Some(colon) = text.find(':') else { return };
    let name = text[..colon].trim();
    if name.is_empty() || name.contains(char::is_whitespace) {
        return;
    }
    let raw = &text[colon + 1..];
    let lead = raw.len() - raw.trim_start().len();
    let trimmed = raw.trim();
    let v_start = start + colon + 1 + lead;
    let full_end = v_start + trimmed.len();
    let (value, important, v_end) = match trimmed.rfind('!') {
        Some(i) if trimmed[i + 1..].trim().eq_ignore_ascii_case("important") => {
            let v = trimmed[..i].trim_end();
            (v, true, v_start + v.len())
        }
        _ => (trimmed, false, full_end),
    };
    out.push(CssDecl {
        name: name.to_string(),
        value: value.to_string(),
        important,
        range: start..full_end,
        value_range: v_start..v_end,
    });
}

// ---------- Edición ----------

fn valid_prop_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '-')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Los valores no pueden cerrar la regla ni la etiqueta `<style>`.
fn valid_value(value: &str) -> bool {
    !value.trim().is_empty() && !value.contains(['{', '}', ';', '<'])
}

/// Cambia, añade (`Some`) o quita (`None`) una declaración de `rule`. Devuelve la hoja
/// nueva; `None` si el nombre o el valor no son aceptables.
pub fn set_decl(css: &str, rule: &CssRule, name: &str, value: Option<&str>) -> Option<String> {
    if !valid_prop_name(name) {
        return None;
    }
    let existing = rule.decls.iter().rfind(|d| d.name.eq_ignore_ascii_case(name));
    let splice = |range: Range<usize>, text: &str| {
        let mut out = String::with_capacity(css.len() + text.len());
        out.push_str(&css[..range.start]);
        out.push_str(text);
        out.push_str(&css[range.end..]);
        out
    };
    match (existing, value) {
        (Some(d), Some(v)) if valid_value(v) => Some(splice(d.value_range.clone(), v.trim())),
        (None, Some(v)) if valid_value(v) => Some(add_decl(css, rule, name, v.trim())),
        (Some(d), None) => Some(remove_decl(css, d)),
        (None, None) => Some(css.to_string()),
        _ => None,
    }
}

fn add_decl(css: &str, rule: &CssRule, name: &str, value: &str) -> String {
    let (at, text) = match rule.decls.last() {
        None => (rule.block.start, format!(" {name}: {value}; ")),
        Some(last) => {
            let line_start = css[..last.range.start].rfind('\n').map_or(0, |i| i + 1);
            let lead = &css[line_start..last.range.start];
            // Si la última declaración abre su línea, la nueva sigue su sangría; si no
            // (regla en una sola línea), va a continuación en la misma línea.
            let sep = if line_start >= rule.block.start && lead.trim().is_empty() {
                format!("\n{lead}")
            } else {
                " ".to_string()
            };
            let rest = &css[last.range.end..rule.block.end];
            let skipped = rest.len() - rest.trim_start().len();
            if rest[skipped..].starts_with(';') {
                (last.range.end + skipped + 1, format!("{sep}{name}: {value};"))
            } else {
                (last.range.end, format!(";{sep}{name}: {value}"))
            }
        }
    };
    format!("{}{}{}", &css[..at], text, &css[at..])
}

fn remove_decl(css: &str, d: &CssDecl) -> String {
    let bytes = css.as_bytes();
    let mut end = d.range.end;
    while bytes.get(end).is_some_and(|b| matches!(b, b' ' | b'\t')) {
        end += 1;
    }
    if bytes.get(end) == Some(&b';') {
        end += 1;
    }
    let mut start = d.range.start;
    // Si la declaración ocupaba su línea entera, se quita también la línea.
    let mut line_start = start;
    while line_start > 0 && matches!(bytes[line_start - 1], b' ' | b'\t') {
        line_start -= 1;
    }
    let mut line_end = end;
    while bytes.get(line_end).is_some_and(|b| matches!(b, b' ' | b'\t' | b'\r')) {
        line_end += 1;
    }
    let own_line = (line_start == 0 || bytes[line_start - 1] == b'\n') && bytes.get(line_end) == Some(&b'\n');
    if own_line {
        start = line_start;
        end = line_end + 1;
    } else {
        // En línea: se lleva también el espacio que lo precede.
        while start > 0 && matches!(bytes[start - 1], b' ' | b'\t') {
            start -= 1;
        }
    }
    format!("{}{}", &css[..start], &css[end..])
}

/// Las reglas de un texto CSS tal como las muestra el panel. `style` es el `<style>` de
/// donde sale (0 para un archivo externo, que se identifica con `file`).
pub fn rule_views_for(css: &str, style: NodeId, file: Option<&str>) -> Vec<RuleView> {
    parse_rules(css)
        .into_iter()
        .enumerate()
        .map(|(index, rule)| RuleView {
            file: file.map(str::to_string),
            style,
            index,
            selector: rule.selector,
            context: rule.context,
            decls: rule
                .decls
                .into_iter()
                .map(|d| DeclView { name: d.name, value: d.value, important: d.important })
                .collect(),
        })
        .collect()
}

// ---------- Acceso desde el documento ----------

impl Document {
    /// Elementos `<style>` conectados al documento y su CSS, en orden de aparición.
    pub fn style_blocks(&self) -> Vec<(NodeId, String)> {
        self.element_tree()
            .into_iter()
            .filter(|(id, _)| self.tag(*id).as_deref() == Some("style"))
            .filter_map(|(id, _)| Some((id, self.text(id)?)))
            .collect()
    }

    /// Hojas de estilo del documento, `<style>` y `<link rel="stylesheet">`, en orden de
    /// aparición (el orden en que se aplica la cascada a igualdad de especificidad).
    pub fn sheets(&self) -> Vec<Sheet> {
        let mut out = Vec::new();
        for (id, _) in self.element_tree() {
            match self.tag(id).as_deref() {
                Some("style") => {
                    if let Some(css) = self.text(id) {
                        out.push(Sheet::Inline { style: id, css });
                    }
                }
                Some("link") => {
                    let is_sheet = self.attr(id, "rel").is_some_and(|rel| {
                        rel.split_whitespace().any(|w| w.eq_ignore_ascii_case("stylesheet"))
                    });
                    let href = self.attr(id, "href").map(|h| h.trim().to_string()).filter(|h| !h.is_empty());
                    if let (true, Some(href)) = (is_sheet, href) {
                        out.push(Sheet::Linked { href });
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Reglas de las hojas `<style>` del documento, como las muestra el panel. Las de
    /// archivos externos las añade quien sabe leerlos (`rule_views_for`).
    pub fn rule_views(&self) -> Vec<RuleView> {
        self.style_blocks()
            .into_iter()
            .flat_map(|(style, css)| rule_views_for(&css, style, None))
            .collect()
    }

    /// CSS nuevo de la hoja `style` tras cambiar una declaración de la regla `index`.
    pub fn css_with_decl(&self, style: NodeId, index: usize, name: &str, value: Option<&str>) -> Option<String> {
        let css = self.text(style).filter(|_| self.tag(style).as_deref() == Some("style"))?;
        let rules = parse_rules(&css);
        set_decl(&css, rules.get(index)?, name, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHEET: &str = "/* base */\nbody { margin: 0 }\n\n.hero,\n.big {\n  color: #fff;\n  background: url(\"a;b.png\") no-repeat !important;\n  --x: calc(1px + 2px);\n}\n@media (max-width: 600px) {\n  .hero { color: red; }\n}\n@font-face { font-family: X; src: url(x.woff); }\n@import url(other.css);\np::before { content: \"a;b}\" }\n";

    fn rules() -> Vec<CssRule> {
        parse_rules(SHEET)
    }

    #[test]
    fn reads_rules_with_selectors_context_and_declarations() {
        let r = rules();
        let sel: Vec<_> = r.iter().map(|r| r.selector.as_str()).collect();
        assert_eq!(sel, ["body", ".hero,\n.big", ".hero", "p::before"]);
        assert_eq!(r[2].context.as_deref(), Some("@media (max-width: 600px)"));
        assert_eq!(r[0].context, None);
        let names: Vec<_> = r[1].decls.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["color", "background", "--x"]);
        let bg = &r[1].decls[1];
        assert_eq!(bg.value, "url(\"a;b.png\") no-repeat");
        assert!(bg.important);
        assert_eq!(&SHEET[bg.value_range.clone()], bg.value);
        assert_eq!(r[1].decls[2].value, "calc(1px + 2px)");
        assert_eq!(r[3].decls[0].value, "\"a;b}\"");
    }

    #[test]
    fn ranges_point_at_the_original_text() {
        for rule in rules() {
            for d in &rule.decls {
                assert!(SHEET[d.range.clone()].starts_with(&d.name), "{d:?}");
                assert_eq!(&SHEET[d.value_range.clone()], d.value);
            }
            assert!(SHEET[rule.block.clone()].len() <= SHEET.len());
        }
    }

    #[test]
    fn changing_a_value_touches_only_that_value() {
        let r = rules();
        let out = set_decl(SHEET, &r[0], "margin", Some("8px")).unwrap();
        assert_eq!(out, SHEET.replace("margin: 0 }", "margin: 8px }"));
        let out = set_decl(SHEET, &r[1], "BACKGROUND", Some("none")).unwrap();
        assert!(out.contains("background: none !important;"));
    }

    #[test]
    fn adding_follows_the_local_style() {
        let r = rules();
        // Regla de una sola línea: sin `;` final.
        let out = set_decl(SHEET, &r[0], "padding", Some("4px")).unwrap();
        assert!(out.contains("body { margin: 0; padding: 4px }"), "{out}");
        // Regla multilínea: misma sangría, tras el último `;`.
        let out = set_decl(SHEET, &r[1], "margin", Some("1em")).unwrap();
        assert!(out.contains("  --x: calc(1px + 2px);\n  margin: 1em;\n}"), "{out}");
    }

    #[test]
    fn adding_to_an_empty_rule() {
        let css = "a {}";
        let r = parse_rules(css);
        assert_eq!(set_decl(css, &r[0], "color", Some("red")).unwrap(), "a { color: red; }");
    }

    #[test]
    fn removing_drops_the_whole_line_or_just_the_declaration() {
        let r = rules();
        let out = set_decl(SHEET, &r[1], "color", None).unwrap();
        assert!(out.contains(".big {\n  background:"), "{out}");
        let out = set_decl(SHEET, &r[0], "margin", None).unwrap();
        assert!(out.contains("body { }") || out.contains("body {}"), "{out}");
        // Quitar algo que no existe no cambia nada.
        assert_eq!(set_decl(SHEET, &r[0], "color", None).unwrap(), SHEET);
    }

    #[test]
    fn edited_sheets_parse_back_with_the_expected_values() {
        let r = rules();
        let out = set_decl(SHEET, &r[1], "margin", Some("1em")).unwrap();
        let again = parse_rules(&out);
        assert_eq!(again.len(), r.len());
        assert!(again[1].decls.iter().any(|d| d.name == "margin" && d.value == "1em"));
        assert_eq!(again[3].decls[0].value, r[3].decls[0].value);
    }

    #[test]
    fn rejects_values_that_could_break_out_of_the_rule() {
        let r = rules();
        for bad in ["red; } body { display:none", "a}b", "</style><script>", "  "] {
            assert!(set_decl(SHEET, &r[0], "color", Some(bad)).is_none(), "{bad}");
        }
        assert!(set_decl(SHEET, &r[0], "bad name", Some("x")).is_none());
        assert!(set_decl(SHEET, &r[0], "1x", Some("x")).is_none());
    }

    #[test]
    fn nested_rules_and_garbage_do_not_panic() {
        for css in [
            "",
            "{",
            "a {",
            "a { color: red",
            "a { b: c; &:hover { d: e } f: g }",
            "@media { a { b: c } ",
            "}}} a{b:c}",
            "a { b: url(",
            "a { b: \"unterminated",
            "é { ü: ñ }",
        ] {
            for rule in parse_rules(css) {
                let _ = set_decl(css, &rule, "color", Some("red"));
                let _ = set_decl(css, &rule, "color", None);
            }
        }
        let r = parse_rules("a { b: c; &:hover { d: e } f: g }");
        let names: Vec<_> = r[0].decls.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["b", "f"]);
    }

    #[test]
    fn editing_a_declaration_is_saved_preserving_the_rest_of_the_file() {
        use crate::command::{History, SetText};
        use crate::source::SaveMode;
        let src = "<style>\n  h1   { color: red; }\n  /* nota */\n</style>\n<h1 id=a>x</h1>";
        let mut doc = Document::parse(src);
        let views = doc.rule_views();
        let css = doc.css_with_decl(views[0].style, 0, "color", Some("blue")).unwrap();
        History::default().execute(&mut doc, Box::new(SetText::new(views[0].style, css))).unwrap();
        let saved = doc.to_saved_html();
        assert_eq!(saved.mode, SaveMode::Preserved);
        assert_eq!(saved.html, src.replace("color: red", "color: blue"));
    }

    #[test]
    fn sheets_lists_style_and_link_in_document_order() {
        let doc = Document::parse(
            "<head><link rel=\"stylesheet\" href=\"a.css\"><style>p{}</style>\
             <LINK REL='preload stylesheet' HREF=' css/b.css '><link rel=icon href=x.ico>\
             <link rel=stylesheet></head><body></body>",
        );
        let kinds: Vec<String> = doc
            .sheets()
            .into_iter()
            .map(|s| match s {
                Sheet::Inline { css, .. } => format!("style:{css}"),
                Sheet::Linked { href } => format!("link:{href}"),
            })
            .collect();
        assert_eq!(kinds, ["link:a.css", "style:p{}", "link:css/b.css"]);
    }

    #[test]
    fn rule_views_for_marks_the_source_file() {
        let v = rule_views_for("a { color: red }", 0, Some("css/x.css"));
        assert_eq!(v[0].file.as_deref(), Some("css/x.css"));
        assert_eq!(v[0].index, 0);
        assert_eq!(Document::parse("<style>a{b:c}</style>").rule_views()[0].file, None);
    }

    #[test]
    fn document_exposes_style_blocks_and_edits() {
        let doc = Document::parse("<style>h1 { color: red }</style><h1>x</h1>");
        let views = doc.rule_views();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].selector, "h1");
        assert_eq!(views[0].decls[0].value, "red");
        let css = doc.css_with_decl(views[0].style, 0, "color", Some("blue")).unwrap();
        assert_eq!(css, "h1 { color: blue }");
        assert!(doc.css_with_decl(doc.find_tag("h1").unwrap(), 0, "color", Some("x")).is_none());
    }
}
