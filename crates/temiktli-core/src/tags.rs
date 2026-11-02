//! Catálogo de etiquetas que se pueden insertar y dónde cabe cada una. Una `<td>` solo vale
//! dentro de una `<tr>`, un `<option>` dentro de un `<select>`, `<title>` en el `<head>`…:
//! `insertion_point` decide el sitio según lo que esté seleccionado, o explica por qué no se
//! puede. Así lo que se inserta es siempre HTML que el navegador no va a reorganizar.

use crate::dom::{Document, is_void};
use crate::protocol::NodeId;
use crate::t;
use serde::Serialize;

/// Un subgrupo dentro de una categoría larga: nombre y sus etiquetas.
pub type Group = (&'static str, &'static [&'static str]);

/// Una categoría del menú «Insertar etiquetas». Si es larga se reparte en `groups` (y `tags`
/// queda vacío); si no, sus etiquetas están directamente en `tags`. `name` (y el primer
/// elemento de cada grupo) es la clave del texto en los archivos de idioma, no el texto:
/// `menu_json` lo traduce.
#[derive(Serialize, Debug)]
pub struct Category {
    pub name: &'static str,
    pub tags: &'static [&'static str],
    pub groups: &'static [Group],
}

impl Category {
    /// Todas sus etiquetas, estén o no repartidas en subgrupos.
    pub fn all_tags(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.tags.iter().copied().chain(self.groups.iter().flat_map(|g| g.1.iter().copied()))
    }
}

const fn flat(name: &'static str, tags: &'static [&'static str]) -> Category {
    Category { name, tags, groups: &[] }
}

/// Categorías del menú «Insertar etiquetas», en el orden en que se muestran.
pub const CATEGORIES: &[Category] = &[
    flat("menu.tags.document", &["html", "head", "body", "title", "base", "meta", "link", "style"]),
    flat("menu.tags.structure", &["header", "nav", "main", "section", "article", "aside", "footer", "address", "hgroup", "search"]),
    flat("menu.tags.headings", &["h1", "h2", "h3", "h4", "h5", "h6"]),
    flat("menu.tags.text", &["p", "div", "span", "pre", "blockquote", "hr"]),
    // Con 24 etiquetas en una sola lista no cabía en pantalla: se reparte por para qué sirven.
    Category {
        name: "menu.tags.format",
        tags: &[],
        groups: &[
            ("menu.tags.format_emphasis", &["b", "strong", "em", "i", "mark", "u", "s", "small"]),
            ("menu.tags.format_code", &["code", "kbd", "samp", "var", "data", "time"]),
            ("menu.tags.format_quotes", &["cite", "dfn", "abbr"]),
            ("menu.tags.format_position", &["sub", "sup", "bdi", "bdo"]),
            ("menu.tags.format_links", &["a", "br", "wbr"]),
        ],
    },
    flat("menu.tags.lists", &["ul", "ol", "li", "menu", "dl", "dt", "dd"]),
    flat("menu.tags.links", &["a", "area", "map"]),
    flat("menu.tags.images", &["img", "picture", "source", "figure", "figcaption", "canvas"]),
    flat("menu.tags.media", &["audio", "video", "track"]),
    flat("menu.tags.embedded", &["iframe", "embed", "object", "param"]),
    flat("menu.tags.tables", &["table", "caption", "colgroup", "col", "thead", "tbody", "tfoot", "tr", "th", "td"]),
    Category {
        name: "menu.tags.forms",
        tags: &[],
        groups: &[
            ("menu.tags.forms_fields", &["input", "textarea", "select", "option", "optgroup", "datalist"]),
            ("menu.tags.forms_structure", &["form", "fieldset", "legend", "label"]),
            ("menu.tags.forms_buttons", &["button", "output", "progress", "meter"]),
        ],
    },
    flat("menu.tags.interactive", &["details", "summary", "dialog"]),
    flat("menu.tags.scripts", &["script", "noscript"]),
    flat("menu.tags.edits", &["del", "ins"]),
    flat("menu.tags.components", &["template", "slot"]),
];

/// El catálogo para el menú, con los nombres de categorías y subgrupos ya en el idioma
/// activo: `[{ name, tags, groups: [[name, tags]] }]`.
pub fn menu_json() -> serde_json::Value {
    let cats: Vec<_> = CATEGORIES
        .iter()
        .map(|c| {
            let groups: Vec<_> = c.groups.iter().map(|(k, tags)| serde_json::json!([t(k), tags])).collect();
            serde_json::json!({ "name": t(c.name), "tags": c.tags, "groups": groups })
        })
        .collect();
    serde_json::Value::Array(cats)
}

/// ¿Es una etiqueta que el editor sabe crear?
pub fn is_insertable(tag: &str) -> bool {
    CATEGORIES.iter().any(|c| c.all_tags().any(|t| t == tag))
}

/// Etiquetas que solo pueden aparecer dentro de otras concretas.
const ONLY_INSIDE: &[(&str, &[&str])] = &[
    ("li", &["ul", "ol", "menu"]),
    ("dt", &["dl"]),
    ("dd", &["dl"]),
    ("option", &["select", "datalist", "optgroup"]),
    ("optgroup", &["select"]),
    ("tr", &["table", "thead", "tbody", "tfoot"]),
    ("th", &["tr"]),
    ("td", &["tr"]),
    ("thead", &["table"]),
    ("tbody", &["table"]),
    ("tfoot", &["table"]),
    ("caption", &["table"]),
    ("colgroup", &["table"]),
    ("col", &["colgroup"]),
    ("legend", &["fieldset"]),
    ("summary", &["details"]),
    ("figcaption", &["figure"]),
    ("source", &["picture", "audio", "video"]),
    ("track", &["audio", "video"]),
    ("param", &["object"]),
    ("area", &["map"]),
];

/// Etiquetas de «texto en línea»: caben dentro de un párrafo, un enlace o un título.
const PHRASING: &[&str] = &[
    "a", "abbr", "b", "bdi", "bdo", "br", "cite", "code", "data", "dfn", "em", "i", "kbd", "mark", "s", "samp",
    "small", "strong", "sub", "sup", "time", "u", "var", "wbr", "span", "img", "button", "input", "select",
    "textarea", "label", "output", "progress", "meter", "del", "ins", "canvas", "audio", "video", "iframe",
    "embed", "object", "picture", "map", "slot", "script", "noscript", "template",
];

/// Elementos que solo admiten texto en línea dentro: un bloque no cabe, va después.
const TEXT_ONLY: &[&str] = &[
    "p", "h1", "h2", "h3", "h4", "h5", "h6", "span", "a", "button", "label", "legend", "pre", "output", "option",
    "textarea", "script", "style", "title", "abbr", "b", "bdi", "bdo", "cite", "code", "data", "dfn", "em", "i",
    "kbd", "mark", "s", "samp", "small", "strong", "sub", "sup", "time", "u", "var", "del", "ins", "summary",
    "dt", "caption", "figcaption",
];

/// Elementos cuyos hijos tienen que ser otros concretos: ahí no se mete un bloque cualquiera.
/// ¿Puede `<child>` ir directamente dentro de `<parent>`? Las reglas de anidación que el
/// navegador aplicaría (o corregiría reorganizando el documento). `Err` dice por qué no.
pub(crate) fn accepts_child(parent: &str, child: &str) -> Result<(), String> {
    if matches!(child, "html" | "head" | "body") {
        return Err(t!("nesting.cannot_move_child", child = child));
    }
    if is_void(parent) {
        return Err(t!("nesting.void_parent", parent = parent));
    }
    let head_only = matches!(child, "title" | "base" | "meta" | "link" | "style");
    if head_only && parent != "head" {
        return Err(t!("nesting.head_only", child = child));
    }
    let only = |allowed: &[&str]| -> Result<(), String> {
        if allowed.contains(&child) {
            Ok(())
        } else {
            let names: Vec<String> = allowed.iter().map(|tag| format!("<{tag}>")).collect();
            Err(t!("nesting.parent_accepts", parent = parent, list = names.join(&t("units.list_comma"))))
        }
    };
    match parent {
        "html" => return Err(t("nesting.html_children")),
        "head" if !head_only && !matches!(child, "script" | "noscript" | "template") => {
            return Err(t("nesting.head_children"));
        }
        "table" => only(&["caption", "colgroup", "thead", "tbody", "tfoot", "tr"])?,
        "thead" | "tbody" | "tfoot" => only(&["tr"])?,
        "tr" => only(&["td", "th"])?,
        "colgroup" => only(&["col"])?,
        "select" => only(&["option", "optgroup"])?,
        "optgroup" | "datalist" => only(&["option"])?,
        "ul" | "ol" | "menu" => only(&["li"])?,
        "dl" => only(&["dt", "dd", "div"])?,
        "picture" => only(&["source", "img"])?,
        "audio" | "video" => only(&["source", "track"])?,
        "map" => only(&["area"])?,
        "hgroup" => only(&["h1", "h2", "h3", "h4", "h5", "h6", "p"])?,
        _ => {}
    }
    if let Some((_, parents)) = ONLY_INSIDE.iter().find(|(t, _)| *t == child) {
        if !parents.contains(&parent) {
            let names: Vec<String> = parents.iter().map(|p| format!("<{p}>")).collect();
            return Err(t!("nesting.child_goes_inside", child = child, list = names.join(&t("units.list_or"))));
        }
    }
    if is_text_only_tag(parent) && !matches!(parent, "a" | "ins" | "del") && !is_phrasing_tag(child) {
        return Err(t!("nesting.text_only_parent", parent = parent, child = child));
    }
    Ok(())
}

fn accept_into(doc: &Document, parent: NodeId, child_tag: &str) -> Result<(), String> {
    let parent_tag = doc.tag(parent).ok_or_else(|| t("nesting.cannot_drop_here"))?;
    accepts_child(&parent_tag, child_tag)
}

/// ¿Es una etiqueta «en línea» (cuyo espacio alrededor se ve)?
pub(crate) fn is_phrasing_tag(tag: &str) -> bool {
    PHRASING.contains(&tag)
}

/// ¿Es una etiqueta que solo lleva texto y elementos en línea?
pub(crate) fn is_text_only_tag(tag: &str) -> bool {
    TEXT_ONLY.contains(&tag)
}

const STRUCTURAL: &[&str] = &[
    "table", "thead", "tbody", "tfoot", "tr", "colgroup", "select", "optgroup", "datalist", "ul", "ol", "menu",
    "dl", "picture", "audio", "video", "map", "object", "hgroup", "html", "head",
];

impl Document {
    /// Dónde cae `id` si se suelta `place` ("before", "after" o "inside") del elemento `target`:
    /// `(padre, posición)`, con la posición contada con `id` todavía en su sitio (como la ve el
    /// lienzo). `Err` explica por qué no se puede: reglas de anidación, o soltar un elemento
    /// dentro de sí mismo.
    pub fn resolve_drop(&self, id: NodeId, target: NodeId, place: &str) -> Result<(NodeId, usize), String> {
        let tag = self.tag(id).filter(|_| self.is_attached(id)).ok_or_else(|| t("nesting.element_gone"))?;
        if matches!(tag.as_str(), "html" | "head" | "body") {
            return Err(t("nesting.cannot_move_root"));
        }
        if !self.is_attached(target) {
            return Err(t("nesting.cannot_drop_here"));
        }
        let (parent, index) = match place {
            "inside" => {
                // Sobre <html> se entiende «dentro del cuerpo».
                let into = match self.tag(target).as_deref() {
                    Some("html") => self.find_tag("body").ok_or_else(|| t("nesting.no_body"))?,
                    _ => target,
                };
                (into, self.child_count(into))
            }
            "before" | "after" => {
                let (p, i) = self.parent_and_index(target).ok_or_else(|| t("nesting.cannot_drop_here"))?;
                (p, if place == "before" { i } else { i + 1 })
            }
            other => return Err(t!("nesting.unknown_place", place = other)),
        };
        // Ni dentro de sí mismo ni de sus descendientes.
        let mut cur = Some(parent);
        while let Some(c) = cur {
            if c == id {
                return Err(t("nesting.inside_itself"));
            }
            cur = self.parent_and_index(c).map(|(p, _)| p);
        }
        accept_into(self, parent, &tag)?;
        Ok((parent, index))
    }

    /// Dónde insertar un elemento `tag` teniendo `selection` seleccionado: `(padre, posición)`.
    /// `Err` explica en español por qué no se puede (y qué hacer).
    pub fn insertion_point(&self, tag: &str, selection: Option<NodeId>) -> Result<(NodeId, usize), String> {
        let body = self.find_tag("body").ok_or_else(|| t("nesting.no_body"))?;
        let sel = selection.filter(|&id| self.is_attached(id)).unwrap_or(body);
        let sel_tag = self.tag(sel).unwrap_or_default();
        let end = |n: NodeId| (n, self.child_count(n));

        if matches!(tag, "html" | "head" | "body") {
            return Err(t!("nesting.already_has", tag = tag));
        }

        // El <head>: título, base, meta, enlaces y estilos.
        if matches!(tag, "title" | "base" | "meta" | "link" | "style") {
            let head = self.find_tag("head").ok_or_else(|| t("nesting.no_head"))?;
            if matches!(tag, "title" | "base") && self.child_tags(head).iter().any(|child| child == tag) {
                return Err(t!("nesting.head_already_has", tag = tag));
            }
            return Ok(end(head));
        }

        // Etiquetas que solo valen dentro de otras: se sube desde la selección hasta encontrarlas.
        if let Some((_, parents)) = ONLY_INSIDE.iter().find(|(t, _)| *t == tag) {
            let mut n = sel;
            loop {
                let n_tag = self.tag(n).unwrap_or_default();
                if parents.contains(&n_tag.as_str()) {
                    return self.inside(tag, n, &n_tag);
                }
                let Some((p, i)) = self.parent_and_index(n) else { break };
                if self.tag(p).is_some_and(|t| parents.contains(&t.as_str())) {
                    return Ok((p, i + 1));
                }
                n = p;
            }
            let names: Vec<String> = parents.iter().map(|p| format!("<{p}>")).collect();
            return Err(t!("nesting.select_parent", tag = tag, list = names.join(&t("units.list_or"))));
        }

        // El resto va en el cuerpo. Desde el <head> se pasa al final del cuerpo.
        let in_head = self.path(sel).is_some_and(|p| p.starts_with("html > head") || p == "html");
        if in_head {
            return Ok(end(body));
        }
        let phrasing = PHRASING.contains(&tag);
        let takes_children = !is_void(&sel_tag)
            && !STRUCTURAL.contains(&sel_tag.as_str())
            && (phrasing || !TEXT_ONLY.contains(&sel_tag.as_str()));
        if takes_children {
            return Ok(end(sel));
        }
        // No cabe dentro: va después, subiendo mientras el hueco tampoco admita contenido.
        let mut n = sel;
        loop {
            let (p, i) = self.parent_and_index(n).ok_or_else(|| t("nesting.nowhere_to_insert"))?;
            let p_tag = self.tag(p).unwrap_or_default();
            if STRUCTURAL.contains(&p_tag.as_str()) && p_tag != "html" {
                n = p;
            } else if p_tag == "html" || p_tag == "head" {
                return Ok(end(body));
            } else {
                return Ok((p, i + 1));
            }
        }
    }

    /// Dentro de `parent` (que es de uno de los tipos válidos): al final, salvo los casos en
    /// que el orden o una capa intermedia importan.
    fn inside(&self, tag: &str, parent: NodeId, parent_tag: &str) -> Result<(NodeId, usize), String> {
        match (tag, parent_tag) {
            // Una fila nunca cuelga directa de <table>: el navegador la metería en un <tbody>.
            ("tr", "table") => {
                let last = (0..self.child_count(parent))
                    .rev()
                    .filter_map(|i| self.child_at(parent, i))
                    .find(|&c| self.tag(c).as_deref() == Some("tbody"))
                    .ok_or_else(|| t("nesting.table_without_tbody"))?;
                Ok((last, self.child_count(last)))
            }
            // El título de la tabla es lo primero.
            ("caption", "table") => Ok((parent, 0)),
            _ => Ok((parent, self.child_count(parent))),
        }
    }

    /// Nombres de las etiquetas de los hijos elemento de `parent`.
    fn child_tags(&self, parent: NodeId) -> Vec<String> {
        (0..self.child_count(parent))
            .filter_map(|i| self.child_at(parent, i))
            .filter_map(|c| self.tag(c))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r##"<!DOCTYPE html><html><head><title>T</title></head><body>
<main><p id="p">Texto <b id="b">negrita</b></p><div id="d"></div>
<ul id="ul"><li id="li">uno</li></ul><dl id="dl"><dt>t</dt><dd>d</dd></dl>
<table id="table"><thead id="thead"><tr id="trh"><th>H</th></tr></thead><tbody id="tbody"><tr id="tr"><td id="td">c</td></tr></tbody></table>
<select id="select"><option id="option">a</option></select>
<fieldset id="fieldset"><legend>l</legend></fieldset><details id="details"><summary>s</summary></details>
<figure id="figure"><img src="a.png" alt=""></figure>
<picture id="picture"><img src="a.png" alt=""></picture><video id="video"></video>
<object id="object"></object><map name="m" id="map"></map>
<img id="img" src="b.png" alt=""></main></body></html>"##;

    fn id(d: &Document, dom_id: &str) -> NodeId {
        (1..500).find(|&n| d.attr(n, "id").as_deref() == Some(dom_id)).unwrap_or_else(|| panic!("no hay #{dom_id}"))
    }

    fn put(d: &Document, tag: &str, sel: &str) -> Result<(String, usize), String> {
        let (p, i) = d.insertion_point(tag, Some(id(d, sel)))?;
        Ok((d.tag(p).unwrap(), i))
    }

    #[test]
    fn the_catalog_is_the_requested_one_and_every_tag_is_unique_per_category() {
        let names: Vec<&str> = CATEGORIES.iter().map(|c| c.name).collect();
        assert_eq!(names.len(), 16);
        assert_eq!(names[0], "menu.tags.document");
        assert_eq!(names[15], "menu.tags.components");
        // Los nombres son claves: el menú recibe el texto traducido (inglés en los tests).
        let menu = menu_json();
        assert_eq!(menu[0]["name"], "Document");
        assert_eq!(menu[15]["name"], "Web Components");
        assert_eq!(menu[0]["tags"][0], "html");
        let total: usize = CATEGORIES.iter().map(|c| c.all_tags().count()).sum();
        assert_eq!(total, 8 + 10 + 6 + 6 + 24 + 7 + 3 + 6 + 3 + 4 + 10 + 14 + 3 + 2 + 2 + 2);
        for c in CATEGORIES {
            let mut seen = std::collections::HashSet::new();
            assert!(c.all_tags().all(|t| seen.insert(t)), "repetida en {}", c.name);
            // Una categoría reparte sus etiquetas en subgrupos, o las tiene sueltas; no las dos cosas.
            assert!(c.tags.is_empty() || c.groups.is_empty(), "{} mezcla etiquetas y subgrupos", c.name);
        }
        assert!(is_insertable("canvas") && !is_insertable("canva") && !is_insertable("script2"));
    }

    #[test]
    fn format_is_split_into_short_groups_and_keeps_all_its_tags() {
        let (pos, formato) = CATEGORIES.iter().enumerate().find(|(_, c)| c.name == "menu.tags.format").unwrap();
        assert!(formato.tags.is_empty());
        let names: Vec<&str> = formato.groups.iter().map(|g| g.0).collect();
        assert_eq!(
            names,
            ["menu.tags.format_emphasis", "menu.tags.format_code", "menu.tags.format_quotes", "menu.tags.format_position", "menu.tags.format_links"]
        );
        // Ningún subgrupo es largo (ese era el problema de una sola lista de 24).
        assert!(formato.groups.iter().all(|g| g.1.len() <= 8));
        // Y siguen estando exactamente las 24 etiquetas pedidas, ni una más ni una menos.
        let mut got: Vec<&str> = formato.all_tags().collect();
        got.sort_unstable();
        let mut want = vec![
            "a", "abbr", "b", "bdi", "bdo", "br", "cite", "code", "data", "dfn", "em", "i", "kbd", "mark", "s",
            "samp", "small", "strong", "sub", "sup", "time", "u", "var", "wbr",
        ];
        want.sort_unstable();
        assert_eq!(got, want);
        // Se serializa para el menú con sus subgrupos, con los nombres traducidos.
        let json = &menu_json()[pos];
        assert_eq!(json["name"], "Formatting");
        assert_eq!(json["groups"][0][0], "Emphasis and style");
        assert_eq!(json["groups"][0][1][0], "b");
    }

    #[test]
    fn forms_are_split_into_short_groups_and_keep_all_their_tags() {
        let forms = CATEGORIES.iter().find(|c| c.name == "menu.tags.forms").unwrap();
        assert!(forms.tags.is_empty());
        let names: Vec<&str> = forms.groups.iter().map(|g| g.0).collect();
        assert_eq!(names, ["menu.tags.forms_fields", "menu.tags.forms_structure", "menu.tags.forms_buttons"]);
        assert!(forms.groups.iter().all(|g| g.1.len() <= 8));
        let mut got: Vec<&str> = forms.all_tags().collect();
        got.sort_unstable();
        let mut want = vec![
            "form", "input", "textarea", "select", "option", "optgroup", "button", "label", "fieldset", "legend",
            "datalist", "output", "progress", "meter",
        ];
        want.sort_unstable();
        assert_eq!(got, want);
    }

    #[test]
    fn no_category_has_a_long_flat_list_except_the_ones_that_are_still_short() {
        // El menú no debe volver a tener listas largas de golpe: más de 10 etiquetas sueltas,
        // se reparten en subgrupos.
        for c in CATEGORIES {
            assert!(c.tags.len() <= 10, "{} tiene {} etiquetas sueltas: divídela en subgrupos", c.name, c.tags.len());
        }
    }

    #[test]
    fn the_document_tags_that_already_exist_are_refused() {
        let d = Document::parse(DOC);
        for t in ["html", "head", "body"] {
            assert!(d.insertion_point(t, None).unwrap_err().contains("there can be only one"), "{t}");
        }
        // <title> ya existe en el <head>: no hay dos. <meta> y <link> sí pueden repetirse.
        assert!(d.insertion_point("title", None).is_err());
        assert!(d.insertion_point("base", None).is_ok());
        let head = d.find_tag("head").unwrap();
        assert_eq!(d.insertion_point("meta", None).unwrap(), (head, d.child_count(head)));
        assert_eq!(d.insertion_point("style", Some(id(&d, "p"))).unwrap().0, head);
    }

    #[test]
    fn block_content_goes_inside_containers_and_after_text_only_elements() {
        let d = Document::parse(DOC);
        assert_eq!(put(&d, "section", "d").unwrap().0, "div");
        // Un bloque no cabe dentro de un párrafo: va después, en su contenedor.
        let (parent, index) = d.insertion_point("section", Some(id(&d, "p"))).unwrap();
        assert_eq!(d.tag(parent).as_deref(), Some("main"));
        assert_eq!(index, d.parent_and_index(id(&d, "p")).unwrap().1 + 1);
        // Texto en línea sí cabe dentro del párrafo (y de la negrita).
        assert_eq!(put(&d, "em", "p").unwrap().0, "p");
        assert_eq!(put(&d, "em", "b").unwrap().0, "b");
        // Tras un elemento vacío por definición (imagen) va después.
        assert_eq!(put(&d, "p", "img").unwrap().0, "main");
        // Nada se mete directamente en una lista ni en una tabla: se sube.
        assert_eq!(put(&d, "div", "ul").unwrap().0, "main");
        assert_eq!(put(&d, "div", "td").unwrap().0, "td");
        assert_eq!(put(&d, "div", "tr").unwrap().0, "main");
        assert_eq!(put(&d, "div", "select").unwrap().0, "main");
    }

    #[test]
    fn tags_that_need_a_parent_find_it_or_explain() {
        let d = Document::parse(DOC);
        // <li>: dentro de la lista, o después de otro <li>.
        assert_eq!(put(&d, "li", "ul").unwrap(), ("ul".into(), 1));
        let (p, i) = d.insertion_point("li", Some(id(&d, "li"))).unwrap();
        assert_eq!((d.tag(p).as_deref(), i), (Some("ul"), 1));
        // Una celda después de otra celda; desde dentro de una celda también.
        assert_eq!(put(&d, "td", "td").unwrap().0, "tr");
        // Una fila nunca directa en <table>: en su último <tbody>.
        assert_eq!(put(&d, "tr", "table").unwrap().0, "tbody");
        assert_eq!(put(&d, "tr", "tr").unwrap().0, "tbody");
        assert_eq!(put(&d, "tr", "thead").unwrap().0, "thead");
        // El título de la tabla es lo primero.
        assert_eq!(put(&d, "caption", "table").unwrap(), ("table".into(), 0));
        // Una opción, una leyenda, un resumen, una leyenda de figura, una fuente, un área.
        assert_eq!(put(&d, "option", "select").unwrap().0, "select");
        assert_eq!(put(&d, "legend", "fieldset").unwrap().0, "fieldset");
        assert_eq!(put(&d, "summary", "details").unwrap().0, "details");
        assert_eq!(put(&d, "figcaption", "figure").unwrap().0, "figure");
        assert_eq!(put(&d, "source", "picture").unwrap().0, "picture");
        assert_eq!(put(&d, "track", "video").unwrap().0, "video");
        assert_eq!(put(&d, "param", "object").unwrap().0, "object");
        assert_eq!(put(&d, "area", "map").unwrap().0, "map");
        assert_eq!(put(&d, "dd", "dl").unwrap().0, "dl");
        // Si no hay dónde, el error dice qué seleccionar.
        let e = d.insertion_point("td", Some(id(&d, "d"))).unwrap_err();
        assert!(e.contains("<tr>"), "{e}");
        let e = d.insertion_point("li", Some(id(&d, "p"))).unwrap_err();
        assert!(e.contains("<ul>") && e.contains("<ol>"), "{e}");
        // Una tabla sin <tbody> pide uno.
        let t = Document::parse("<table id=\"t\"></table>");
        assert!(t.insertion_point("tr", Some(id(&t, "t"))).unwrap_err().contains("<tbody>"));
    }

    #[test]
    fn with_nothing_selected_things_go_at_the_end_of_the_body() {
        let d = Document::parse("<!DOCTYPE html><html><head></head><body><p>x</p></body></html>");
        let body = d.find_tag("body").unwrap();
        assert_eq!(d.insertion_point("div", None).unwrap(), (body, 1));
        // Seleccionado algo del <head> (o <html>): también al final del cuerpo.
        let head = d.find_tag("head").unwrap();
        assert_eq!(d.insertion_point("div", Some(head)).unwrap(), (body, 1));
        assert_eq!(d.insertion_point("div", d.find_tag("html")).unwrap(), (body, 1));
    }
}
