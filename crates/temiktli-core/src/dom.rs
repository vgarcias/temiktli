//! Documento HTML editable. El árbol `RcDom` es la fuente de verdad; cada elemento
//! lleva un `NodeId` estable, también visible en el lienzo como `data-tk-id`.

use crate::protocol::{Crumb, NodeId, SelectionInfo};
use crate::source::SourceMap;
use html5ever::serialize::{SerializeOpts, TraversalScope};
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::{Attribute, LocalName, ParseOpts, QualName, ns, parse_document, serialize};
use markup5ever_rcdom::{Handle, Node, NodeData, RcDom, SerializableHandle};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Atributo que enlaza cada elemento del lienzo con su nodo en Rust.
/// Nunca debe aparecer en un archivo guardado.
pub const ID_ATTR: &str = "data-tk-id";

const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Tamaño máximo del interior que se ofrece para editar en Propiedades (se reenvía con cada
/// cambio); más allá, se edita en la vista de código.
const MAX_INNER_HTML: usize = 20_000;

/// SVG gris de 200x100 para que una imagen nueva sea visible y seleccionable.
const PLACEHOLDER_IMG: &str = "data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' \
width='200' height='100'><rect width='200' height='100' fill='%23ccd'/></svg>";

pub fn is_void(tag: &str) -> bool {
    VOID_TAGS.contains(&tag)
}

/// Nombre de atributo seguro para escribir en el DOM: empieza por letra y solo
/// contiene letras, dígitos y `- _ : .`. Excluye el marcador interno.
pub fn valid_attr_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
        && !name.eq_ignore_ascii_case(ID_ATTR)
}

pub struct Document {
    pub(crate) dom: RcDom,
    pub(crate) nodes: HashMap<NodeId, Handle>,
    next: NodeId,
    /// Texto original con la posición de cada etiqueta, para guardar sin reformatear.
    pub(crate) source: Option<SourceMap>,
}

impl Document {
    /// Parsea HTML y marca cada elemento con `data-tk-id`.
    pub fn parse(html: &str) -> Self {
        let mut doc = Self::parse_plain(html);
        doc.source = SourceMap::build(html, &doc);
        doc
    }

    /// Como `parse`, sin localizar etiquetas (más barato; para comparar resultados).
    pub(crate) fn parse_plain(html: &str) -> Self {
        let dom = parse_document(RcDom::default(), ParseOpts::default())
            .from_utf8()
            .one(html.as_bytes());
        let mut doc = Document { dom, nodes: HashMap::new(), next: 0, source: None };
        let root = doc.dom.document.clone();
        doc.annotate(&root);
        doc
    }

    fn annotate(&mut self, node: &Handle) {
        if matches!(node.data, NodeData::Element { .. }) {
            self.register(node);
        }
        // Copia: `register` no toca `children`, pero evita mantener el préstamo.
        let children: Vec<Handle> = node.children.borrow().iter().cloned().collect();
        for child in &children {
            self.annotate(child);
        }
    }

    /// Asigna un id nuevo al elemento y lo escribe como atributo.
    fn register(&mut self, node: &Handle) -> NodeId {
        self.next += 1;
        let id = self.next;
        if let NodeData::Element { attrs, .. } = &node.data {
            attrs.borrow_mut().push(Attribute {
                name: QualName::new(None, ns!(), LocalName::from(ID_ATTR)),
                value: id.to_string().into(),
            });
        }
        self.nodes.insert(id, node.clone());
        id
    }

    // ---------- Consulta ----------

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    pub fn tag(&self, id: NodeId) -> Option<String> {
        match &self.nodes.get(&id)?.data {
            NodeData::Element { name, .. } => Some(name.local.to_string()),
            _ => None,
        }
    }

    /// Ruta `html > body > div.hero > h1`, calculada al vuelo (siempre al día).
    pub fn path(&self, id: NodeId) -> Option<String> {
        let labels: Vec<String> = self.crumbs(id)?.into_iter().map(|c| c.label).collect();
        Some(labels.join(" > "))
    }

    /// La ruta de `html` hasta `id`, un tramo por elemento (`div.hero`) con su id: cada uno se
    /// puede seleccionar.
    pub fn crumbs(&self, id: NodeId) -> Option<Vec<Crumb>> {
        let mut segs = Vec::new();
        let mut cur = self.nodes.get(&id)?.clone();
        loop {
            if let NodeData::Element { name, attrs, .. } = &cur.data {
                let class = attrs
                    .borrow()
                    .iter()
                    .find(|a| &*a.name.local == "class")
                    .and_then(|a| a.value.split_whitespace().next().map(str::to_string));
                let label = match class {
                    Some(c) => format!("{}.{c}", name.local),
                    None => name.local.to_string(),
                };
                if let Some(cid) = self.id_of(&cur) {
                    segs.push(Crumb { id: cid, label });
                }
            }
            match parent_of(&cur) {
                Some(p) => cur = p,
                None => break,
            }
        }
        segs.reverse();
        Some(segs)
    }

    pub fn attrs(&self, id: NodeId) -> Vec<(String, String)> {
        match self.nodes.get(&id).map(|n| &n.data) {
            Some(NodeData::Element { attrs, .. }) => attrs
                .borrow()
                .iter()
                .filter(|a| &*a.name.local != ID_ATTR)
                .map(|a| (a.name.local.to_string(), a.value.to_string()))
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn attr(&self, id: NodeId, name: &str) -> Option<String> {
        self.attrs(id).into_iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    /// Texto del elemento si todos sus hijos son texto; `None` si tiene elementos
    /// hijos, comentarios, o es un elemento vacío por definición (`<img>`).
    pub fn text(&self, id: NodeId) -> Option<String> {
        let node = self.nodes.get(&id)?;
        let NodeData::Element { name, .. } = &node.data else { return None };
        if is_void(&name.local) {
            return None;
        }
        let mut out = String::new();
        for child in node.children.borrow().iter() {
            match &child.data {
                NodeData::Text { contents } => out.push_str(&contents.borrow()),
                _ => return None,
            }
        }
        Some(out)
    }

    pub fn parent_and_index(&self, id: NodeId) -> Option<(NodeId, usize)> {
        let node = self.nodes.get(&id)?;
        let parent = parent_of(node)?;
        let index = parent.children.borrow().iter().position(|c| Rc::ptr_eq(c, node))?;
        let pid = self.id_of(&parent)?;
        Some((pid, index))
    }

    pub fn child_count(&self, id: NodeId) -> usize {
        self.nodes.get(&id).map_or(0, |n| n.children.borrow().len())
    }

    /// Id del hijo en `index`, si ese hijo es un elemento.
    pub fn child_at(&self, parent: NodeId, index: usize) -> Option<NodeId> {
        let child = self.nodes.get(&parent)?.children.borrow().get(index)?.clone();
        self.id_of(&child)
    }

    /// Primer elemento con esa etiqueta, en orden de creación de ids.
    pub fn find_tag(&self, tag: &str) -> Option<NodeId> {
        let mut ids: Vec<NodeId> = self.nodes.keys().copied().collect();
        ids.sort_unstable();
        ids.into_iter().find(|&id| self.tag(id).as_deref() == Some(tag))
    }

    /// ¿Sigue colgado del documento? Falso para nodos borrados o aún sin insertar.
    pub fn is_attached(&self, id: NodeId) -> bool {
        let Some(node) = self.nodes.get(&id) else { return false };
        let mut cur = node.clone();
        while let Some(p) = parent_of(&cur) {
            cur = p;
        }
        Rc::ptr_eq(&cur, &self.dom.document)
    }

    fn id_of(&self, handle: &Handle) -> Option<NodeId> {
        self.nodes.iter().find(|(_, h)| Rc::ptr_eq(h, handle)).map(|(id, _)| *id)
    }

    /// HTML del interior del elemento, sin rastros del editor. `None` en los elementos vacíos
    /// por definición (`<br>`, `<img>`).
    pub fn inner_html(&self, id: NodeId) -> Option<String> {
        let node = self.nodes.get(&id)?;
        if is_void(&self.tag(id)?) {
            return None;
        }
        let handle: SerializableHandle = node.clone().into();
        let mut out = Vec::new();
        let opts = SerializeOpts { traversal_scope: TraversalScope::ChildrenOnly(None), ..Default::default() };
        serialize(&mut out, &handle, opts).ok()?;
        Some(strip_ids(&String::from_utf8(out).ok()?))
    }

    pub fn selection_info(&self, id: NodeId) -> Option<SelectionInfo> {
        let tag = self.tag(id)?;
        let can_delete = !matches!(tag.as_str(), "html" | "head" | "body");
        let text = self.text(id);
        // El interior en HTML solo se ofrece si el cuadro de texto simple no sirve (hay
        // elementos dentro), y si no es tan grande que enviarlo en cada cambio estorbe.
        let (inner_html, inner_large) = match (&text, can_delete) {
            (None, true) => match self.inner_html(id) {
                Some(h) if h.len() > MAX_INNER_HTML => (None, true),
                other => (other, false),
            },
            _ => (None, false),
        };
        Some(SelectionInfo {
            id,
            path: self.path(id)?,
            crumbs: self.crumbs(id)?,
            attrs: self.attrs(id),
            text,
            inner_html,
            inner_large,
            can_delete,
            tag,
        })
    }

    // ---------- Mutación (la usan los comandos) ----------

    /// Cambia o quita un atributo. Devuelve el valor anterior.
    pub fn set_attr(&mut self, id: NodeId, name: &str, value: Option<String>) -> Option<String> {
        let node = self.nodes.get(&id)?;
        let NodeData::Element { attrs, .. } = &node.data else { return None };
        let name = name.to_ascii_lowercase();
        let mut attrs = attrs.borrow_mut();
        let pos = attrs.iter().position(|a| *a.name.local == *name);
        let prev = pos.map(|i| attrs[i].value.to_string());
        match (pos, value) {
            (Some(i), Some(v)) => attrs[i].value = v.into(),
            (Some(i), None) => {
                attrs.remove(i);
            }
            (None, Some(v)) => attrs.push(Attribute {
                name: QualName::new(None, ns!(), LocalName::from(name.as_str())),
                value: v.into(),
            }),
            (None, None) => {}
        }
        prev
    }

    /// Reemplaza el contenido por un único nodo de texto. Devuelve el texto anterior.
    pub fn set_text(&mut self, id: NodeId, text: &str) -> Option<String> {
        let prev = self.text(id)?;
        let node = self.nodes.get(&id)?.clone();
        let mut children = node.children.borrow_mut();
        children.clear();
        if !text.is_empty() {
            let t = Node::new(NodeData::Text { contents: RefCell::new(StrTendril::from(text)) });
            t.parent.set(Some(Rc::downgrade(&node)));
            children.push(t);
        }
        Some(prev)
    }

    /// Crea un elemento (y sus hijos por defecto) sin colocarlo en el árbol.
    pub fn create_element(&mut self, tag: &str) -> Option<NodeId> {
        // El documento ya tiene html, head y body, y solo puede haber uno de cada.
        if !crate::tags::is_insertable(tag) || matches!(tag, "html" | "head" | "body") {
            return None;
        }
        // Un elemento nuevo con su contenido por defecto: lo justo para que se vea, se pueda
        // seleccionar y sea HTML que el navegador no reorganice al abrirlo de nuevo (por eso
        // una tabla lleva su <tbody>, y un <select>, sus <option>). Los textos salen del
        // bloque `content` del idioma activo.
        let d = |k: &str| crate::i18n::t(&format!("content.defaults.{k}"));
        let c = |k: &str| crate::i18n::t(&format!("content.{k}"));
        let (item, term, desc, cell, header) = (d("list_item"), d("term"), d("description"), d("cell"), d("header_cell"));
        let (option, opt1, opt2, group, group_fields) = (d("option"), d("option_1"), d("option_2"), d("group"), d("group_fields"));
        let (summary, details, dialog, legend, heading) = (d("summary"), d("details_body"), d("dialog"), d("caption_figure"), d("heading"));
        let (subheading, form_fields, link_text, button, text) = (d("subheading"), d("form_fields"), d("link"), d("button"), d("text"));
        let (abbr_title, abbr, data, time, pre) = (d("abbr_title"), d("abbr"), d("data"), d("time"), d("preformatted"));
        let (caption, label, slot, style, script) = (d("table_caption"), d("label"), d("slot"), d("style"), d("script"));
        let (noscript, object, placeholder, block) = (d("noscript"), d("object"), d("input_placeholder"), d("new_block"));
        let (page_title, iframe_title, stylesheet, srclang, track_label, map_name, list_id) = (
            c("title_placeholder"), c("iframe_title"), c("stylesheet_href"), c("track_srclang"), c("track_label"),
            c("map_name"), c("datalist_id"),
        );
        let handle = match tag {
            "ul" | "ol" | "menu" => self.with_children(tag, &[], &[("li", &[], Some(item.as_str())), ("li", &[], Some(item.as_str()))]),
            "dl" => self.with_children(tag, &[], &[("dt", &[], Some(term.as_str())), ("dd", &[], Some(desc.as_str()))]),
            "table" => {
                let row = self.with_children("tr", &[], &[("td", &[], Some(cell.as_str())), ("td", &[], Some(cell.as_str()))]);
                let body = self.new_el("tbody", &[], None);
                link(&body, usize::MAX, &row);
                let row = self.with_children("tr", &[], &[("td", &[], Some(cell.as_str())), ("td", &[], Some(cell.as_str()))]);
                link(&body, usize::MAX, &row);
                let table = self.new_el("table", &[("border", "1")], None);
                link(&table, usize::MAX, &body);
                table
            }
            "thead" => {
                let row = self.with_children("tr", &[], &[("th", &[], Some(header.as_str())), ("th", &[], Some(header.as_str()))]);
                let head = self.new_el("thead", &[], None);
                link(&head, usize::MAX, &row);
                head
            }
            "tbody" | "tfoot" => {
                let row = self.with_children("tr", &[], &[("td", &[], Some(cell.as_str())), ("td", &[], Some(cell.as_str()))]);
                let part = self.new_el(tag, &[], None);
                link(&part, usize::MAX, &row);
                part
            }
            "tr" => self.with_children(tag, &[], &[("td", &[], Some(cell.as_str())), ("td", &[], Some(cell.as_str()))]),
            "colgroup" => self.with_children(tag, &[], &[("col", &[], None), ("col", &[], None)]),
            "select" => self.with_children(tag, &[("name", "")], &[("option", &[], Some(opt1.as_str())), ("option", &[], Some(opt2.as_str()))]),
            "optgroup" => self.with_children(tag, &[("label", group.as_str())], &[("option", &[], Some(opt1.as_str())), ("option", &[], Some(opt2.as_str()))]),
            "datalist" => self.with_children(tag, &[("id", list_id.as_str())], &[("option", &[("value", opt1.as_str())], None), ("option", &[("value", opt2.as_str())], None)]),
            "fieldset" => self.with_children(tag, &[], &[("legend", &[], Some(group.as_str())), ("p", &[], Some(group_fields.as_str()))]),
            "details" => self.with_children(tag, &[], &[("summary", &[], Some(summary.as_str())), ("p", &[], Some(details.as_str()))]),
            "dialog" => self.with_children(tag, &[("open", "")], &[("p", &[], Some(dialog.as_str()))]),
            "figure" => self.with_children(tag, &[], &[("img", &[("src", PLACEHOLDER_IMG), ("alt", "")], None), ("figcaption", &[], Some(legend.as_str()))]),
            "picture" => self.with_children(tag, &[], &[("img", &[("src", PLACEHOLDER_IMG), ("alt", "")], None)]),
            "map" => self.with_children(tag, &[("name", map_name.as_str())], &[("area", &[("shape", "rect"), ("coords", "0,0,50,50"), ("href", "#"), ("alt", "")], None)]),
            "hgroup" => self.with_children(tag, &[], &[("h2", &[], Some(heading.as_str())), ("p", &[], Some(subheading.as_str()))]),
            "form" => self.with_children(tag, &[("action", "#")], &[("p", &[], Some(form_fields.as_str()))]),
            "a" => self.new_el("a", &[("href", "#")], Some(link_text.as_str())),
            "img" => self.new_el("img", &[("src", PLACEHOLDER_IMG), ("alt", "")], None),
            "button" => self.new_el("button", &[], Some(button.as_str())),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => self.new_el(tag, &[], Some(heading.as_str())),
            "p" | "span" | "b" | "bdi" | "cite" | "code" | "dfn" | "em" | "i" | "kbd" | "mark" | "s" | "samp"
            | "small" | "strong" | "sub" | "sup" | "u" | "var" | "del" | "ins" => self.new_el(tag, &[], Some(text.as_str())),
            "abbr" => self.new_el(tag, &[("title", abbr_title.as_str())], Some(abbr.as_str())),
            "bdo" => self.new_el(tag, &[("dir", "ltr")], Some(text.as_str())),
            "data" => self.new_el(tag, &[("value", "")], Some(data.as_str())),
            "time" => self.new_el(tag, &[("datetime", "")], Some(time.as_str())),
            "pre" => self.new_el(tag, &[], Some(pre.as_str())),
            "li" => self.new_el(tag, &[], Some(item.as_str())),
            "dt" => self.new_el(tag, &[], Some(term.as_str())),
            "dd" => self.new_el(tag, &[], Some(desc.as_str())),
            "th" => self.new_el(tag, &[], Some(header.as_str())),
            "td" => self.new_el(tag, &[], Some(cell.as_str())),
            "caption" => self.new_el(tag, &[], Some(caption.as_str())),
            "figcaption" | "legend" => self.new_el(tag, &[], Some(if tag == "legend" { group.as_str() } else { legend.as_str() })),
            "summary" => self.new_el(tag, &[], Some(summary.as_str())),
            "option" => self.new_el(tag, &[], Some(option.as_str())),
            "label" => self.new_el(tag, &[], Some(label.as_str())),
            "output" => self.new_el(tag, &[], Some("0")),
            "slot" => self.new_el(tag, &[], Some(slot.as_str())),
            "title" => self.new_el(tag, &[], Some(page_title.as_str())),
            "style" => self.new_el(tag, &[], Some(style.as_str())),
            "script" => self.new_el(tag, &[], Some(script.as_str())),
            // Con JavaScript activado el contenido de <noscript> es texto, no elementos.
            "noscript" => self.new_el(tag, &[], Some(noscript.as_str())),
            "object" => self.new_el(tag, &[("data", "")], Some(object.as_str())),
            "iframe" => self.new_el(tag, &[("src", "about:blank"), ("title", iframe_title.as_str())], None),
            "audio" | "video" => self.new_el(tag, &[("controls", "")], None),
            "canvas" => self.new_el(tag, &[("width", "300"), ("height", "150")], None),
            "progress" => self.new_el(tag, &[("value", "50"), ("max", "100")], None),
            "meter" => self.new_el(tag, &[("value", "50"), ("min", "0"), ("max", "100")], None),
            "input" => self.new_el(tag, &[("type", "text"), ("name", ""), ("placeholder", placeholder.as_str())], None),
            "textarea" => self.new_el(tag, &[("rows", "3"), ("name", "")], None),
            "meta" => self.new_el(tag, &[("name", "description"), ("content", "")], None),
            "link" => self.new_el(tag, &[("rel", "stylesheet"), ("href", stylesheet.as_str())], None),
            "track" => self.new_el(tag, &[("kind", "subtitles"), ("srclang", srclang.as_str()), ("label", track_label.as_str())], None),
            "param" => self.new_el(tag, &[("name", ""), ("value", "")], None),
            "embed" => self.new_el(tag, &[("src", "")], None),
            "hr" | "br" | "wbr" | "col" | "source" | "base" | "area" | "template" => self.new_el(tag, &[], None),
            // Contenedores: un párrafo dentro, para que sean visibles y seleccionables.
            _ => {
                let c = self.new_el(tag, &[], None);
                let p = self.new_el("p", &[], Some(block.as_str()));
                link(&c, usize::MAX, &p);
                c
            }
        };
        self.id_of(&handle)
    }

    /// Un elemento con sus hijos directos: `(etiqueta, atributos, texto)` cada uno.
    fn with_children(
        &mut self,
        tag: &str,
        attrs: &[(&str, &str)],
        children: &[(&str, &[(&str, &str)], Option<&str>)],
    ) -> Handle {
        let parent = self.new_el(tag, attrs, None);
        for (t, a, text) in children {
            let child = self.new_el(t, a, *text);
            link(&parent, usize::MAX, &child);
        }
        parent
    }

    fn new_el(&mut self, tag: &str, attrs: &[(&str, &str)], text: Option<&str>) -> Handle {
        let node = Node::new(NodeData::Element {
            name: QualName::new(None, ns!(html), LocalName::from(tag)),
            attrs: RefCell::new(
                attrs
                    .iter()
                    .map(|(k, v)| Attribute {
                        name: QualName::new(None, ns!(), LocalName::from(*k)),
                        value: (*v).into(),
                    })
                    .collect(),
            ),
            template_contents: RefCell::new(None),
            mathml_annotation_xml_integration_point: false,
        });
        self.register(&node);
        if let Some(t) = text {
            let id = self.id_of(&node).expect("recién registrado");
            self.set_text(id, t);
        }
        node
    }

    /// Cuelga `id` bajo `parent` en `index` (se recorta al máximo). Devuelve el índice real.
    pub fn insert(&mut self, parent: NodeId, index: usize, id: NodeId) -> Option<usize> {
        let p = self.nodes.get(&parent)?.clone();
        let c = self.nodes.get(&id)?.clone();
        if is_void(&self.tag(parent)?) || is_ancestor_or_self(&c, &p) {
            return None;
        }
        Some(link(&p, index, &c))
    }

    /// Descuelga `id` de su padre. Devuelve dónde estaba, para poder restaurarlo.
    pub fn detach(&mut self, id: NodeId) -> Option<(NodeId, usize)> {
        let (parent, index) = self.parent_and_index(id)?;
        let node = self.nodes.get(&id)?.clone();
        let p = self.nodes.get(&parent)?.clone();
        p.children.borrow_mut().remove(index);
        node.parent.set(None);
        Some((parent, index))
    }

    // ---------- Salida ----------

    /// HTML de un subárbol, con `data-tk-id` (para parchear el lienzo).
    pub fn subtree_html(&self, id: NodeId) -> Option<String> {
        let handle: SerializableHandle = self.nodes.get(&id)?.clone().into();
        let mut out = Vec::new();
        let opts = SerializeOpts { traversal_scope: TraversalScope::IncludeNode, ..Default::default() };
        serialize(&mut out, &handle, opts).ok()?;
        String::from_utf8(out).ok()
    }

    /// HTML para el lienzo (con `data-tk-id`).
    pub fn to_canvas_html(&self) -> String {
        self.serialize_all(false)
    }

    /// HTML limpio para guardar (sin rastros del editor).
    pub fn to_clean_html(&self) -> String {
        self.serialize_all(true)
    }

    fn serialize_all(&self, strip: bool) -> String {
        let mut out = Vec::new();
        let handle: SerializableHandle = self.dom.document.clone().into();
        serialize(&mut out, &handle, Default::default()).expect("serialize");
        let s = String::from_utf8(out).expect("utf8");
        if strip { strip_ids(&s) } else { s }
    }
}

pub(crate) fn parent_of(node: &Handle) -> Option<Handle> {
    let weak = node.parent.take();
    let parent = weak.as_ref().and_then(|w| w.upgrade());
    node.parent.set(weak);
    parent
}

fn is_ancestor_or_self(candidate: &Handle, of: &Handle) -> bool {
    let mut cur = Some(of.clone());
    while let Some(n) = cur {
        if Rc::ptr_eq(&n, candidate) {
            return true;
        }
        cur = parent_of(&n);
    }
    false
}

fn link(parent: &Handle, index: usize, child: &Handle) -> usize {
    let mut children = parent.children.borrow_mut();
    let index = index.min(children.len());
    children.insert(index, child.clone());
    child.parent.set(Some(Rc::downgrade(parent)));
    index
}

/// Quita ` data-tk-id="N"` del texto serializado. Segura porque el serializador
/// siempre emite el atributo con comillas dobles y el valor es numérico.
pub(crate) fn strip_ids(s: &str) -> String {
    let needle = format!(" {ID_ATTR}=\"");
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(&needle) {
        out.push_str(&rest[..i]);
        let after = &rest[i + needle.len()..];
        match after.find('"') {
            Some(j) => rest = &after[j + 1..],
            None => {
                rest = after;
                break;
            }
        }
    }
    out.push_str(rest);
    out
}
