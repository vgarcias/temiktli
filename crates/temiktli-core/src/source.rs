//! Guardar sin reformatear: en vez de reserializar todo el árbol, se parchea el texto
//! original solo donde el documento cambió, y el resultado se verifica antes de usarlo.
//!
//! Flujo: `SourceMap::build` conoce, para cada elemento original, dónde están sus
//! etiquetas y hasta dónde llega. `render` compara el árbol actual con esa foto y
//! produce ediciones de texto. `Document::to_saved_html` re-parsea el resultado y solo
//! lo da por bueno si equivale exactamente al árbol editado; si no, cae al guardado
//! normalizado de siempre (nunca peor que antes).

use crate::dom::{Document, ID_ATTR, is_void, strip_ids};
use crate::t;
use crate::pretty::{
    detect_eol, detect_unit, format_fragment, gap_around, is_raw, line_indent, starts_line, whitespace_free_parent,
    whole_lines,
};
use crate::protocol::NodeId;
use crate::tags::is_phrasing_tag;
use crate::spanparse::{ParsedElem, TagScan, parse_spans};
use markup5ever_rcdom::{Handle, NodeData};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;

/// Resultado de `Document::to_saved_html`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedHtml {
    pub html: String,
    pub mode: SaveMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveMode {
    /// El texto original se conservó; solo cambió lo editado.
    Preserved,
    /// Se reescribió el archivo entero con el formato normalizado del parser.
    Normalized(String),
}

/// Cuántos elementos crea `fragment` al colocarlo en el cuerpo de una página. Si el parser
/// descarta alguno (una celda suelta fuera de una tabla), no se cuenta: el recuento deja de
/// cuadrar al pegarlo y se rechaza, que es lo que se quiere.
pub fn fragment_elements(fragment: &str) -> usize {
    let page = format!("<!DOCTYPE html><html><head></head><body>{fragment}</body></html>");
    // html, head y body ya estaban: no son del fragmento.
    Document::parse_plain(&page).element_tree().len().saturating_sub(3)
}

/// Elementos cuyo contenido es texto crudo: sin su cierre, se tragan todo lo que sigue.
const RAW_TEXT: &[&str] = &["script", "style", "textarea", "title", "xmp", "iframe", "noembed", "noframes", "plaintext"];

/// ¿Se puede insertar este HTML dentro de un elemento sin descuadrar lo que lo rodea?
fn check_fragment(inner: &str) -> Result<(), String> {
    let spans = parse_spans(inner).ok_or_else(|| t("source.parse_failed"))?;
    let mut open: Vec<&str> = Vec::new();
    for t in &spans.tags {
        let name = t.name.as_str();
        if t.scan.is_end {
            match open.iter().rposition(|o| o.eq_ignore_ascii_case(name)) {
                Some(p) => open.truncate(p),
                None => return Err(t!("source.close_without_open", name = name)),
            }
        } else if !is_void(name) && !t.scan.self_closing {
            open.push(name);
        }
    }
    if let Some(raw) = open.iter().find(|o| RAW_TEXT.iter().any(|r| r.eq_ignore_ascii_case(o))) {
        return Err(t!("source.tag_not_closed", raw = raw));
    }
    Ok(())
}

/// Dónde está un elemento con etiqueta propia en un texto HTML.
pub(crate) struct Span {
    id: NodeId,
    /// Su etiqueta de inicio.
    tag: Range<usize>,
    /// Byte donde termina todo lo que pertenece al elemento.
    end: usize,
    /// Entre la etiqueta de inicio y la de cierre; `None` en los elementos vacíos (`<br>`).
    inner: Option<Range<usize>>,
}

/// Posiciones de los elementos con etiqueta propia en un texto HTML.
pub struct SourceIndex(Vec<Span>);

impl SourceIndex {
    pub fn tag_range(&self, id: NodeId) -> Option<Range<usize>> {
        self.0.iter().find(|s| s.id == id).map(|s| s.tag.clone())
    }

    /// De la etiqueta de inicio al final del elemento (incluido su cierre).
    pub fn outer_range(&self, id: NodeId) -> Option<Range<usize>> {
        let s = self.0.iter().find(|s| s.id == id)?;
        Some(s.tag.start..s.end)
    }

    /// El texto entre la etiqueta de inicio y la de cierre. Si el cierre es implícito
    /// (`<li>` sin `</li>`), llega hasta donde acaba el elemento.
    pub fn inner_range(&self, id: NodeId) -> Option<Range<usize>> {
        self.0.iter().find(|s| s.id == id)?.inner.clone()
    }

    /// El elemento más interno (el que empieza más tarde) que contiene el byte `pos`.
    pub fn element_at(&self, pos: usize) -> Option<NodeId> {
        self.0
            .iter()
            .filter(|s| s.tag.start <= pos && pos < s.end)
            .max_by_key(|s| s.tag.start)
            .map(|s| s.id)
    }
}

/// Lo que se sabía de un elemento cuando se leyó el texto.
pub(crate) struct ElemSrc {
    attrs: Vec<(String, String)>,
    text: Option<String>,
    parent: Option<NodeId>,
    tag: String,
    /// `None` si el elemento es implícito (html/head/body/tbody… sin etiqueta propia).
    start: Option<TagScan>,
    end_tag: Option<Range<usize>>,
    /// Fin de todo lo que pertenece al elemento (cierre explícito o implícito).
    extent_end: usize,
}

pub(crate) struct SourceMap {
    text: String,
    elems: HashMap<NodeId, ElemSrc>,
}

pub(crate) enum ChildKind {
    Elem(NodeId),
    Text,
    Other,
}

impl SourceMap {
    /// `None` si no se pueden localizar las etiquetas con seguridad.
    pub(crate) fn build(src: &str, doc: &Document) -> Option<SourceMap> {
        let spans = parse_spans(src)?;
        let tree = doc.element_tree();
        if tree.len() != spans.elems.len() {
            return None;
        }
        for ((id, _), e) in tree.iter().zip(&spans.elems) {
            if doc.tag(*id)? != e.name {
                return None;
            }
        }

        let n = spans.elems.len();
        let mut start: Vec<Option<usize>> = vec![None; n];
        let mut end_tag: Vec<Option<Range<usize>>> = vec![None; n];
        let mut extent_end = vec![src.len(); n];
        let mut stack: Vec<usize> = Vec::new();

        for (ti, t) in spans.tags.iter().enumerate() {
            let sc = &t.scan;
            if !sc.is_end {
                let Some(e) = t.element else { continue };
                start[e] = Some(ti);
                // Lo que sigue abierto y no contiene a `e` se cerró (implícitamente) aquí.
                while let Some(&top) = stack.last() {
                    if is_ancestor(&spans.elems, top, e) {
                        break;
                    }
                    extent_end[top] = sc.start;
                    stack.pop();
                }
                let el = &spans.elems[e];
                if is_void(&el.name) || (sc.self_closing && el.foreign) {
                    extent_end[e] = sc.end;
                } else {
                    stack.push(e);
                }
            } else if let Some(p) =
                stack.iter().rposition(|&i| spans.elems[i].name.eq_ignore_ascii_case(&t.name))
            {
                for i in stack.split_off(p + 1) {
                    extent_end[i] = sc.start;
                }
                let e = stack.pop().expect("p existe");
                end_tag[e] = Some(sc.start..sc.end);
                extent_end[e] = sc.end;
            }
        }

        let mut elems = HashMap::new();
        for (k, (id, parent)) in tree.iter().enumerate() {
            elems.insert(
                *id,
                ElemSrc {
                    attrs: doc.attrs(*id),
                    text: doc.text(*id),
                    parent: *parent,
                    tag: spans.elems[k].name.clone(),
                    start: start[k].map(|ti| spans.tags[ti].scan.clone()),
                    end_tag: end_tag[k].take(),
                    extent_end: extent_end[k],
                },
            );
        }
        Some(SourceMap { text: src.to_string(), elems })
    }

    /// Por cada elemento con etiqueta propia: su etiqueta, su interior y dónde termina.
    pub(crate) fn ranges(&self) -> Vec<Span> {
        self.elems
            .iter()
            .filter_map(|(id, e)| {
                let s = e.start.as_ref()?;
                let inner = (!is_void(&e.tag)).then(|| {
                    let close = e.end_tag.as_ref().map_or(e.extent_end, |t| t.start);
                    s.end..close.max(s.end)
                });
                Some(Span { id: *id, tag: s.start..s.end, end: e.extent_end, inner })
            })
            .collect()
    }

    /// Texto original con las ediciones aplicadas. `None` si algún cambio no se sabe
    /// trasladar al texto (el llamador cae al guardado normalizado).
    /// El segundo valor dice si algún elemento nuevo se colocó en su propia línea con sangría
    /// (el texto trae entonces espacio en blanco que el árbol en memoria no tiene).
    pub(crate) fn render(&self, doc: &Document) -> Option<(String, bool)> {
        let tree = doc.element_tree();
        let attached: HashMap<NodeId, Option<NodeId>> = tree.iter().copied().collect();
        let mut edits: Vec<Edit> = Vec::new();

        // Elementos que están en otro sitio que en el texto original (movidos o envueltos).
        let mut moved: HashSet<NodeId> = HashSet::new();
        // Los que siguen con su padre pero cambiaron de orden entre sus hermanos: solo los que
        // sobran de la secuencia que sigue en el orden original (el resto no se toca).
        let mut reordered: HashSet<NodeId> = HashSet::new();
        for &pid in attached.keys() {
            if !self.elems.contains_key(&pid) {
                continue;
            }
            let order: Vec<(NodeId, usize)> = doc
                .children_kinds(pid)
                .into_iter()
                .filter_map(|k| match k {
                    ChildKind::Elem(c) => {
                        let e = self.elems.get(&c).filter(|e| e.parent == Some(pid))?;
                        Some((c, e.start.as_ref()?.start))
                    }
                    _ => None,
                })
                .collect();
            let ranks: Vec<usize> = order.iter().map(|o| o.1).collect();
            if ranks.windows(2).all(|w| w[0] < w[1]) {
                continue;
            }
            for (keep, (c, _)) in longest_increasing(&ranks).into_iter().zip(&order) {
                if !keep {
                    reordered.insert(*c);
                }
            }
        }
        // ¿Cambió el espacio en blanco del texto respecto al árbol (líneas cortadas o añadidas)?
        let mut formatted = false;
        let mut ids: Vec<NodeId> = self.elems.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let el = &self.elems[&id];
            let Some(&cur_parent) = attached.get(&id) else {
                // Borrado: basta con quitar el más externo; sus descendientes van dentro.
                if el.parent.is_some_and(|p| attached.contains_key(&p)) {
                    let s = el.start.as_ref()?;
                    let (a, b) = whole_lines(&self.text, s.start, el.extent_end);
                    formatted |= (a, b) != (s.start, el.extent_end);
                    edits.push(Edit::cut(a, b));
                }
                continue;
            };
            if cur_parent != el.parent || reordered.contains(&id) {
                // Movido: se corta de donde estaba y se escribe de nuevo donde cayó. Si su sitio
                // de antes ya no existe, no sabemos expresarlo.
                if !el.parent.is_some_and(|p| attached.contains_key(&p)) {
                    return None;
                }
                let s = el.start.as_ref()?;
                let (a, b) = whole_lines(&self.text, s.start, el.extent_end);
                formatted |= (a, b) != (s.start, el.extent_end);
                edits.push(Edit::cut(a, b));
                moved.insert(id);
                continue;
            }
            let cur_attrs = doc.attrs(id);
            if !same_attrs(&cur_attrs, &el.attrs) {
                attr_edits(&self.text, el, &cur_attrs, &mut edits)?;
            }
            // Si ya no es "solo texto" es porque ganó hijos: eso lo cubre `insert_edits`.
            if let (Some(old), Some(new)) = (&el.text, doc.text(id)) {
                if new != *old {
                    edits.push(text_edit(el, &new)?);
                }
            }
        }

        // Elementos nuevos: se agrupan por su padre original.
        let mut parents: Vec<NodeId> = Vec::new();
        for (id, parent) in &tree {
            if let Some(p) = parent {
                let is_new = !self.elems.contains_key(id) || moved.contains(id);
                if is_new && self.elems.contains_key(p) && !parents.contains(p) {
                    parents.push(*p);
                }
            }
        }
        for p in parents {
            self.insert_edits(doc, p, &moved, &mut edits, &mut formatted)?;
        }
        Some((apply(&self.text, edits)?, formatted))
    }

    /// Dónde y con qué espacio va el elemento nuevo `child` que cae en `at`: si el hueco ya tiene
    /// saltos de línea, en su propia línea con la sangría de sus hermanos (o la del padre más un
    /// nivel), sin dejar líneas vacías. `None` = va pegado, como estaba escrito el texto.
    fn placed(
        &self,
        doc: &Document,
        parent: NodeId,
        kinds: &[ChildKind],
        moved: &HashSet<NodeId>,
        child: NodeId,
        at: usize,
        html: &str,
    ) -> Option<Edit> {
        let pe = &self.elems[&parent];
        if is_raw(&pe.tag) {
            return None;
        }
        // Un elemento en línea dentro de un bloque con texto añadiría un espacio visible.
        if is_phrasing_tag(&doc.tag(child)?) && !whitespace_free_parent(&pe.tag) {
            return None;
        }
        let src = &self.text;
        let (gap, has_newline) = gap_around(src, at);
        if !has_newline {
            return None;
        }
        let unit = detect_unit(src);
        let eol = detect_eol(src);
        let sibling_indent = kinds.iter().find_map(|k| match k {
            ChildKind::Elem(c) if !moved.contains(c) => {
                let s = self.elems.get(c)?.start.as_ref()?;
                starts_line(src, s.start).then(|| line_indent(src, s.start).to_string())
            }
            _ => None,
        });
        let indent = match sibling_indent {
            Some(i) => i,
            None => format!("{}{unit}", line_indent(src, pe.start.as_ref()?.start)),
        };
        let text = format!("{eol}{indent}{}", format_fragment(html, &indent, &unit, eol));
        Some(Edit { start: gap, end: gap, text })
    }

    fn insert_edits(
        &self,
        doc: &Document,
        parent: NodeId,
        moved: &HashSet<NodeId>,
        out: &mut Vec<Edit>,
        formatted: &mut bool,
    ) -> Option<()> {
        let pe = &self.elems[&parent];
        let kinds = doc.children_kinds(parent);
        let content_start = pe.start.as_ref().filter(|_| !is_void(&pe.tag)).map(|s| s.end);
        let content_end = pe.start.as_ref().map(|_| pe.end_tag.as_ref().map_or(pe.extent_end, |r| r.start));

        // `anchor`: dónde va un hermano nuevo que sigue justo tras lo anterior.
        // Si ya pasó texto, va al final de ese hueco: antes del siguiente elemento
        // original, o al final del contenido.
        let mut anchor = content_start;
        let mut after_text = false;
        for (i, kind) in kinds.iter().enumerate() {
            match kind {
                ChildKind::Other => return None,
                ChildKind::Text => after_text = true,
                ChildKind::Elem(c) => {
                    if let Some(ce) = self.elems.get(c).filter(|_| !moved.contains(c)) {
                        ce.start.as_ref()?;
                        anchor = Some(ce.extent_end);
                        after_text = false;
                        continue;
                    }
                    let at = if after_text {
                        let next = kinds[i + 1..].iter().find_map(|k| match k {
                            ChildKind::Elem(c2) => self.elems.get(c2).filter(|_| !moved.contains(c2)),
                            _ => None,
                        });
                        match next {
                            Some(ne) => ne.start.as_ref()?.start,
                            None => content_end?,
                        }
                    } else {
                        anchor?
                    };
                    let html = strip_ids(&doc.subtree_html(*c)?);
                    match self.placed(doc, parent, &kinds, moved, *c, at, &html) {
                        Some(edit) => {
                            *formatted = true;
                            out.push(edit);
                        }
                        None => out.push(Edit { start: at, end: at, text: html }),
                    }
                }
            }
        }
        Some(())
    }
}

/// Cuáles de `ranks` forman la subsecuencia creciente más larga (`true` = se queda donde está).
fn longest_increasing(ranks: &[usize]) -> Vec<bool> {
    let n = ranks.len();
    let mut len = vec![1usize; n];
    let mut prev: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        for j in 0..i {
            if ranks[j] < ranks[i] && len[j] + 1 > len[i] {
                len[i] = len[j] + 1;
                prev[i] = Some(j);
            }
        }
    }
    let mut keep = vec![false; n];
    let mut cur = (0..n).max_by_key(|&i| (len[i], std::cmp::Reverse(i)));
    while let Some(i) = cur {
        keep[i] = true;
        cur = prev[i];
    }
    keep
}

fn is_ancestor(elems: &[ParsedElem], anc: usize, of: usize) -> bool {
    let mut cur = elems[of].parent;
    while let Some(p) = cur {
        if p == anc {
            return true;
        }
        cur = elems[p].parent;
    }
    false
}

// ---------- Ediciones de texto ----------

struct Edit {
    start: usize,
    end: usize,
    text: String,
}

impl Edit {
    fn cut(start: usize, end: usize) -> Edit {
        Edit { start, end, text: String::new() }
    }
}

/// Aplica ediciones que no se pisan. Las inserciones en un mismo punto conservan su orden.
fn apply(src: &str, mut edits: Vec<Edit>) -> Option<String> {
    edits.sort_by_key(|e| (e.start, e.end));
    let mut out = String::with_capacity(src.len());
    let mut cur = 0;
    for e in edits {
        if e.start < cur || e.end < e.start || e.end > src.len() {
            return None;
        }
        if !src.is_char_boundary(e.start) || !src.is_char_boundary(e.end) {
            return None;
        }
        out.push_str(&src[cur..e.start]);
        out.push_str(&e.text);
        cur = e.end;
    }
    out.push_str(&src[cur..]);
    Some(out)
}

fn same_attrs(a: &[(String, String)], b: &[(String, String)]) -> bool {
    let mut a = a.to_vec();
    let mut b = b.to_vec();
    a.sort();
    b.sort();
    a == b
}

fn escape_attr(v: &str) -> String {
    v.replace('&', "&amp;").replace('"', "&quot;")
}

fn escape_text(v: &str) -> String {
    v.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
}

/// Reescribe solo los atributos que cambiaron dentro de la etiqueta de inicio.
fn attr_edits(src: &str, el: &ElemSrc, cur: &[(String, String)], out: &mut Vec<Edit>) -> Option<()> {
    let sc = el.start.as_ref()?;
    let raw = |name: &str| sc.attrs.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, r)| r.clone());
    let mut added = String::new();
    for (name, val) in cur {
        match el.attrs.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            Some((_, old)) if old == val => {}
            Some(_) => {
                let r = raw(name)?;
                out.push(Edit { start: r.start, end: r.end, text: format!("{name}=\"{}\"", escape_attr(val)) });
            }
            None => added.push_str(&format!(" {name}=\"{}\"", escape_attr(val))),
        }
    }
    for (name, _) in &el.attrs {
        if !cur.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
            // Se quita también el espacio que lo precede.
            let r = raw(name)?;
            let mut start = r.start;
            while start > sc.start && is_ws(src.as_bytes()[start - 1]) {
                start -= 1;
            }
            out.push(Edit::cut(start, r.end));
        }
    }
    if !added.is_empty() {
        out.push(Edit { start: sc.close_at, end: sc.close_at, text: added });
    }
    Some(())
}

fn text_edit(el: &ElemSrc, new: &str) -> Option<Edit> {
    if is_void(&el.tag) {
        return None;
    }
    let from = el.start.as_ref()?.end;
    let to = el.end_tag.as_ref().map_or(el.extent_end, |r| r.start);
    if to < from {
        return None;
    }
    let mut text = if matches!(el.tag.as_str(), "script" | "style") {
        new.to_string()
    } else {
        escape_text(new)
    };
    // El parser descarta un salto de línea inicial en estos elementos.
    if matches!(el.tag.as_str(), "pre" | "textarea" | "listing") && text.starts_with('\n') {
        text.insert(0, '\n');
    }
    Some(Edit { start: from, end: to, text })
}

// ---------- Consultas del documento que solo necesita este módulo ----------

impl Document {
    /// Elementos conectados al documento, en preorden, con su elemento padre.
    pub(crate) fn element_tree(&self) -> Vec<(NodeId, Option<NodeId>)> {
        let ptrs = self.pointer_ids();
        let mut out = Vec::new();
        walk_tree(&self.dom.document, None, &ptrs, &mut out);
        out
    }

    fn pointer_ids(&self) -> HashMap<usize, NodeId> {
        self.nodes.iter().map(|(id, h)| (Rc::as_ptr(h) as usize, *id)).collect()
    }

    pub(crate) fn children_kinds(&self, id: NodeId) -> Vec<ChildKind> {
        let Some(node) = self.nodes.get(&id) else { return Vec::new() };
        let ptrs = self.pointer_ids();
        node.children
            .borrow()
            .iter()
            .map(|c| match &c.data {
                NodeData::Element { .. } => match ptrs.get(&(Rc::as_ptr(c) as usize)) {
                    Some(&cid) => ChildKind::Elem(cid),
                    None => ChildKind::Other,
                },
                NodeData::Text { .. } => ChildKind::Text,
                _ => ChildKind::Other,
            })
            .collect()
    }

    /// Texto para guardar: el original con solo lo editado cambiado, o, si eso no se
    /// puede garantizar, el documento completo con el formato normalizado.
    pub fn to_saved_html(&self) -> SavedHtml {
        let normalized = |why: String| SavedHtml { html: self.to_clean_html(), mode: SaveMode::Normalized(why) };
        let Some(source) = &self.source else {
            return normalized(t("status.normalized_unlocated"));
        };
        let Some((html, formatted)) = source.render(self) else {
            return normalized(t("status.normalized_untranslatable"));
        };
        let again = Document::parse_plain(&html);
        // Un elemento puesto en su propia línea añade espacio que el árbol no tiene: en ese
        // caso se compara sin contar el espacio en blanco entre elementos.
        let same = again.fingerprint() == self.fingerprint()
            || (formatted && again.fingerprint_loose() == self.fingerprint_loose());
        if !same {
            return normalized(t("status.normalized_mismatch"));
        }
        SavedHtml { html, mode: SaveMode::Preserved }
    }

    /// Posición estructural de un elemento: el índice de cada ancestro (y el suyo) entre
    /// los elementos hermanos, desde la raíz. Sirve para volver a encontrar «el mismo»
    /// elemento tras reemplazar el documento, cuando los ids ya no valen.
    pub fn structural_key(&self, id: NodeId) -> Option<Vec<usize>> {
        let tree = self.element_tree();
        let parent: HashMap<NodeId, Option<NodeId>> = tree.iter().copied().collect();
        let mut seen: HashMap<Option<NodeId>, usize> = HashMap::new();
        let mut index: HashMap<NodeId, usize> = HashMap::new();
        for (node, p) in &tree {
            let n = seen.entry(*p).or_default();
            index.insert(*node, *n);
            *n += 1;
        }
        let mut key = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            key.push(*index.get(&c)?);
            cur = *parent.get(&c)?;
        }
        key.reverse();
        Some(key)
    }

    /// El elemento en esa posición estructural, si existe en este documento.
    pub fn node_at_key(&self, key: &[usize]) -> Option<NodeId> {
        let tree = self.element_tree();
        let mut cur: Option<NodeId> = None;
        for &k in key {
            cur = Some(tree.iter().filter(|(_, p)| *p == cur).nth(k)?.0);
        }
        cur
    }

    /// Rango en bytes de la etiqueta de inicio de `id` dentro de `html` (el texto que
    /// muestra la vista de código). `None` si el elemento no tiene etiqueta propia
    /// (`html`, `head`, `body` implícitos) o `html` no corresponde a este documento.
    pub fn tag_range(&self, html: &str, id: NodeId) -> Option<Range<usize>> {
        self.source_index(html)?.tag_range(id)
    }

    /// Elemento más interno cuyo texto (de su etiqueta de inicio a su cierre) contiene el
    /// byte `pos` de `html`. `None` si `html` no corresponde a este documento.
    pub fn element_at(&self, html: &str, pos: usize) -> Option<NodeId> {
        self.source_index(html)?.element_at(pos)
    }

    /// El texto `html` (el del documento tal como se muestra) con el interior del elemento `id`
    /// sustituido por `inner`. Solo cambia ese interior: el resto conserva su formato. Se
    /// rechaza lo que rompería el documento: etiquetas de cierre sin apertura (cerrarían el
    /// elemento o sus padres), `<script>` y similares sin cerrar (se tragarían el resto), o
    /// cualquier cambio en la estructura de fuera del elemento.
    pub fn with_inner_html(&self, html: &str, id: NodeId, inner: &str) -> Result<String, String> {
        let index = self.source_index(html).ok_or_else(|| t("source.element_not_located"))?;
        let range = index.inner_range(id).ok_or_else(|| t("source.nothing_to_edit"))?;
        check_fragment(inner)?;

        let mut new = String::with_capacity(html.len() + inner.len());
        new.push_str(&html[..range.start]);
        new.push_str(inner);
        new.push_str(&html[range.end..]);

        // Lo de fuera del elemento no puede cambiar: mismo elemento en el mismo sitio y el
        // mismo número de elementos a su alrededor.
        let key = self.structural_key(id).ok_or_else(|| t("source.element_detached"))?;
        let after = Document::parse_plain(&new);
        let same = after.node_at_key(&key).filter(|&n| after.tag(n) == self.tag(id));
        let outside_before = self.element_tree().len() - self.subtree_len(id);
        let outside_after = same.map(|n| after.element_tree().len() - after.subtree_len(n));
        if outside_after != Some(outside_before) {
            return Err(t("source.breaks_structure"));
        }
        Ok(new)
    }

    /// El texto del elemento `id` tal como está escrito en `html` (de su etiqueta de inicio
    /// a su cierre), y cuántos elementos contiene contándolo a él. Sirve para copiarlo.
    pub fn outer_source(&self, html: &str, id: NodeId) -> Option<(String, usize)> {
        let range = self.source_index(html)?.outer_range(id)?;
        Some((html[range].to_string(), self.subtree_len(id)))
    }

    /// `html` con `fragment` (un elemento copiado antes con `outer_source`) pegado justo
    /// después del elemento `target`; dentro de `<body>`, al final. `elements` es cuántos
    /// elementos tiene lo copiado: si el resultado no suma exactamente esos (porque el parser
    /// descartaría alguno en ese sitio, p. ej. una fila `<tr>` fuera de una tabla), no se pega.
    pub fn with_pasted(&self, html: &str, target: NodeId, fragment: &str, elements: usize) -> Result<String, String> {
        let tag = self.tag(target).ok_or_else(|| t("source.element_detached"))?;
        if matches!(tag.as_str(), "html" | "head") {
            return Err(t("source.paste_select_body"));
        }
        let index = self.source_index(html).ok_or_else(|| t("source.element_not_located"))?;
        let at = if tag == "body" {
            index.inner_range(target).ok_or_else(|| t("source.body_not_located"))?.end
        } else {
            index.outer_range(target).ok_or_else(|| t("source.element_not_found"))?.end
        };
        check_fragment(fragment)?;

        let mut new = String::with_capacity(html.len() + fragment.len() + 1);
        new.push_str(&html[..at]);
        new.push_str(fragment);
        new.push_str(&html[at..]);

        let key = self.structural_key(target).ok_or_else(|| t("source.element_detached"))?;
        let after = Document::parse_plain(&new);
        let same = after.node_at_key(&key).is_some_and(|n| after.tag(n).as_deref() == Some(tag.as_str()));
        if !same || after.element_tree().len() != self.element_tree().len() + elements {
            return Err(t("source.paste_reorganized"));
        }
        Ok(new)
    }

    /// `html` con `fragment` (por ejemplo un `<meta>`) al principio del `<head>`. Si el `<head>` no
    /// está escrito en el archivo pero sí `<html>`, se escribe uno con ello dentro. Solo cambia
    /// ese sitio: el resto conserva su formato.
    pub fn with_head_prepended(&self, html: &str, fragment: &str) -> Result<String, String> {
        check_fragment(fragment)?;
        let index = self.source_index(html).ok_or_else(|| t("source.head_not_located"))?;
        let head = self.find_tag("head").ok_or_else(|| t("source.no_head"))?;
        let (at, wrap) = match index.inner_range(head) {
            Some(r) => (r.start, false),
            None => {
                let root = self.find_tag("html").ok_or_else(|| t("source.no_html"))?;
                let r = index.inner_range(root).ok_or_else(|| t("source.no_head_or_html"))?;
                (r.start, true)
            }
        };
        // Si lo que sigue ya está en otra línea, lo añadido también (con la sangría habitual).
        let on_new_line = html[at..].starts_with('\n') || html[at..].starts_with("\r\n");
        let body = if on_new_line { format!("\n  {fragment}") } else { fragment.to_string() };
        let insert = if wrap { format!("<head>{body}</head>") } else { body };

        let mut new = String::with_capacity(html.len() + insert.len());
        new.push_str(&html[..at]);
        new.push_str(&insert);
        new.push_str(&html[at..]);

        // Debe haber aparecido exactamente lo escrito, nada más. (Un <head> que no estaba escrito
        // ya existía de forma implícita: escribirlo no añade un elemento.)
        let added = parse_spans(fragment).map_or(0, |s| s.tags.iter().filter(|t| !t.scan.is_end).count());
        let expected = self.element_tree().len() + added;
        let after = Document::parse_plain(&new);
        if after.element_tree().len() != expected {
            return Err(t("source.head_prepend_failed"));
        }
        Ok(new)
    }

    /// `html` con `<!DOCTYPE html>` al principio.
    pub fn with_doctype(&self, html: &str) -> Result<String, String> {
        let new = format!("<!DOCTYPE html>\n{}", html.trim_start_matches('\u{feff}'));
        let after = Document::parse_plain(&new);
        if after.element_tree().len() != self.element_tree().len() {
            return Err(t("source.doctype_failed"));
        }
        Ok(new)
    }

    /// Cuántos elementos hay en el subárbol de `id`, contándolo a él.
    fn subtree_len(&self, id: NodeId) -> usize {
        let tree = self.element_tree();
        let parent: HashMap<NodeId, Option<NodeId>> = tree.iter().copied().collect();
        tree.iter()
            .filter(|(n, _)| {
                let mut cur = Some(*n);
                while let Some(c) = cur {
                    if c == id {
                        return true;
                    }
                    cur = parent.get(&c).copied().flatten();
                }
                false
            })
            .count()
    }

    /// Dónde está cada elemento en `html`. Construirlo recorre todo el texto: quien consulte
    /// muchas veces el mismo documento y texto debe guardarlo y reutilizarlo.
    pub fn source_index(&self, html: &str) -> Option<SourceIndex> {
        Some(SourceIndex(SourceMap::build(html, self)?.ranges()))
    }

    /// Tras guardar, el texto guardado pasa a ser el "original" de las próximas ediciones.
    pub fn rebase_source(&mut self, saved: &str) {
        self.source = SourceMap::build(saved, self);
    }

    /// Representación canónica del árbol, para comparar documentos sin que importe el
    /// orden de los atributos ni cómo se partió el texto en nodos.
    pub(crate) fn fingerprint(&self) -> String {
        let mut out = String::new();
        fp(&self.dom.document, &mut out, false);
        out
    }

    /// Como `fingerprint`, sin contar el texto que es solo espacio en blanco.
    pub(crate) fn fingerprint_loose(&self) -> String {
        let mut out = String::new();
        fp(&self.dom.document, &mut out, true);
        out
    }
}

fn walk_tree(
    h: &Handle,
    parent: Option<NodeId>,
    ptrs: &HashMap<usize, NodeId>,
    out: &mut Vec<(NodeId, Option<NodeId>)>,
) {
    let mut me = parent;
    if matches!(h.data, NodeData::Element { .. }) {
        if let Some(&id) = ptrs.get(&(Rc::as_ptr(h) as usize)) {
            out.push((id, parent));
            me = Some(id);
        }
    }
    for c in h.children.borrow().iter() {
        walk_tree(c, me, ptrs, out);
    }
}

/// Con `loose`, el texto que es solo espacio en blanco no cuenta: sirve para comparar un texto
/// al que se le dio sangría con el árbol, que no tiene ese espacio.
fn fp(h: &Handle, out: &mut String, loose: bool) {
    match &h.data {
        NodeData::Text { contents } => {
            let text = contents.borrow();
            if !(loose && text.trim().is_empty()) {
                out.push_str(&text.replace('&', "&amp;").replace('<', "&lt;"));
            }
        }
        NodeData::Comment { contents } => {
            out.push_str("<!--");
            out.push_str(contents);
            out.push_str("-->");
        }
        NodeData::Doctype { name, .. } => {
            out.push_str("<!doctype ");
            out.push_str(name);
            out.push('>');
        }
        NodeData::Element { name, attrs, .. } => {
            let mut a: Vec<(String, String)> = attrs
                .borrow()
                .iter()
                .filter(|a| &*a.name.local != ID_ATTR)
                .map(|a| (a.name.local.to_string(), a.value.to_string()))
                .collect();
            a.sort();
            out.push('<');
            out.push_str(&name.local);
            for (k, v) in a {
                out.push_str(&format!(" {k}=\"{}\"", v.replace('"', "&quot;")));
            }
            out.push('>');
            for c in h.children.borrow().iter() {
                fp(c, out, loose);
            }
            out.push_str("</");
            out.push_str(&name.local);
            out.push('>');
            return;
        }
        _ => {}
    }
    for c in h.children.borrow().iter() {
        fp(c, out, loose);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{DeleteNode, History, InsertElement, ReplaceDocument, SetAttr, SetText};
    use crate::protocol::Patch;

    /// Parsea, aplica ediciones y guarda.
    fn save(src: &str, edit: impl FnOnce(&mut Document, &mut History)) -> SavedHtml {
        let mut doc = Document::parse(src);
        let mut hist = History::default();
        edit(&mut doc, &mut hist);
        doc.to_saved_html()
    }

    fn id(doc: &Document, tag: &str) -> NodeId {
        doc.find_tag(tag).unwrap_or_else(|| panic!("falta <{tag}>"))
    }

    const MESSY: &str = "<!DOCTYPE html>\r\n<HTML lang=es>\r\n<head>\r\n  <meta charset='utf-8'>\r\n  \
        <title>Mi sitio &copy;</title>\r\n  <!-- estilos -->\r\n  <style>\r\n    body   { margin:0 }\r\n  </style>\r\n</head>\r\n\
        <BODY class=inicio>\r\n\r\n  <h1 id='titulo'>Hola&nbsp;mundo</h1>\r\n  <p>Uno sin cerrar\r\n  \
        <p>Dos con <b>negrita</b> y<br/>salto\r\n  <a title=\"a>b\" href=x>enlace</a>\r\n  \
        <input type=text disabled value=\"a &amp; b\">\r\n</BODY>\r\n</HTML>\r\n";

    #[test]
    fn untouched_document_is_byte_identical() {
        let out = save(MESSY, |_, _| {});
        assert_eq!(out.mode, SaveMode::Preserved);
        assert_eq!(out.html, MESSY);
    }

    #[test]
    fn attribute_change_only_touches_that_attribute() {
        let out = save(MESSY, |d, h| {
            let id = id(d, "h1");
            h.execute(d, Box::new(SetAttr::new(id, "id".into(), Some("nuevo".into())))).unwrap();
        });
        assert_eq!(out.mode, SaveMode::Preserved);
        assert_eq!(out.html, MESSY.replace("id='titulo'", "id=\"nuevo\""));
    }

    #[test]
    fn attribute_added_and_removed() {
        let out = save("<img src=a.png alt='x'>", |d, h| {
            let img = id(d, "img");
            h.execute(d, Box::new(SetAttr::new(img, "alt".into(), None))).unwrap();
            h.execute(d, Box::new(SetAttr::new(img, "title".into(), Some("T \"q\"".into())))).unwrap();
        });
        assert_eq!(out.mode, SaveMode::Preserved);
        assert_eq!(out.html, "<img src=a.png title=\"T &quot;q&quot;\">");
    }

    #[test]
    fn text_edit_replaces_only_the_text() {
        let out = save(MESSY, |d, h| {
            let id = id(d, "h1");
            h.execute(d, Box::new(SetText::new(id, "Adiós <todos> & más".into()))).unwrap();
        });
        assert_eq!(out.mode, SaveMode::Preserved);
        assert_eq!(
            out.html,
            MESSY.replace("Hola&nbsp;mundo", "Adiós &lt;todos&gt; &amp; más")
        );
    }

    #[test]
    fn text_edit_on_unclosed_paragraph() {
        let out = save("<p>uno\n<p>dos", |d, h| {
            let id = id(d, "p");
            h.execute(d, Box::new(SetText::new(id, "UNO\n".into()))).unwrap();
        });
        assert_eq!(out.mode, SaveMode::Preserved);
        assert_eq!(out.html, "<p>UNO\n<p>dos");
    }

    #[test]
    fn deleting_an_element_removes_its_whole_extent() {
        let src = "<div>\n  <p>a</p>\n  <p>b</p>\n</div>";
        let out = save(src, |d, h| {
            let id = id(d, "p");
            h.execute(d, Box::new(DeleteNode::new(id))).unwrap();
        });
        assert_eq!(out.mode, SaveMode::Preserved);
        // Se va con su línea entera: no queda una línea vacía con la sangría.
        assert_eq!(out.html, "<div>\n  <p>b</p>\n</div>");
    }

    use crate::command::MoveNode;

    /// Mueve `what` (la n-ésima etiqueta `tag`) a `place` de `target` y guarda.
    fn moved(src: &str, what: (&str, usize), target: (&str, usize), place: &str) -> (SavedHtml, String) {
        let mut doc = Document::parse(src);
        let mut h = History::default();
        let nth = |d: &Document, tag: &str, n: usize| {
            let mut ids: Vec<NodeId> = (1..2000).filter(|&i| d.tag(i).as_deref() == Some(tag)).collect();
            ids.sort_unstable();
            ids[n]
        };
        let (id, t) = (nth(&doc, what.0, what.1), nth(&doc, target.0, target.1));
        let (parent, index) = doc.resolve_drop(id, t, place).expect("destino válido");
        h.execute(&mut doc, Box::new(MoveNode::new(id, parent, index))).unwrap();
        let saved = doc.to_saved_html();
        h.undo(&mut doc);
        (saved, doc.to_saved_html().html)
    }

    #[test]
    fn moving_an_element_rewrites_only_that_element_and_undo_restores_the_file() {
        let src = "<body>\n  <h1>T</h1>\n  <p>a</p>\n  <div>\n    <p>b</p>\n  </div>\n</body>";
        // Dentro de otro contenedor, con su sangría.
        let (saved, undone) = moved(src, ("p", 0), ("div", 0), "inside");
        assert_eq!(saved.mode, SaveMode::Preserved, "{}", saved.html);
        assert_eq!(saved.html, "<body>\n  <h1>T</h1>\n  <div>\n    <p>b</p>\n    <p>a</p>\n  </div>\n</body>");
        assert_eq!(undone, src);
        // Antes de un hermano anterior.
        let (saved, undone) = moved(src, ("div", 0), ("h1", 0), "before");
        assert_eq!(saved.mode, SaveMode::Preserved, "{}", saved.html);
        assert_eq!(saved.html, "<body>\n  <div>\n    <p>b</p>\n  </div>\n  <h1>T</h1>\n  <p>a</p>\n</body>");
        assert_eq!(undone, src);
        // Después de un hermano posterior (en el mismo padre).
        let (saved, _) = moved(src, ("h1", 0), ("p", 0), "after");
        assert_eq!(saved.html, "<body>\n  <p>a</p>\n  <h1>T</h1>\n  <div>\n    <p>b</p>\n  </div>\n</body>");
    }

    #[test]
    fn a_moved_element_keeps_its_attributes_and_children() {
        let src = "<body>\n  <ul>\n    <li class=\"x\">A</li>\n    <li>B</li>\n  </ul>\n</body>";
        let (saved, _) = moved(src, ("li", 0), ("li", 1), "after");
        assert_eq!(saved.mode, SaveMode::Preserved, "{}", saved.html);
        assert_eq!(saved.html, "<body>\n  <ul>\n    <li>B</li>\n    <li class=\"x\">A</li>\n  </ul>\n</body>");
    }

    #[test]
    fn drops_follow_the_html_nesting_rules() {
        let doc = Document::parse(
            "<body><table><tbody><tr><td>x</td></tr></tbody></table><p>y</p><div>d</div><ul><li>a</li><li>b</li></ul></body>",
        );
        let nth = |tag: &str, n: usize| {
            let mut ids: Vec<NodeId> = (1..2000).filter(|&i| doc.tag(i).as_deref() == Some(tag)).collect();
            ids.sort_unstable();
            ids[n]
        };
        let (td, p, div, ul, li0, li1, tr) = (nth("td", 0), nth("p", 0), nth("div", 0), nth("ul", 0), nth("li", 0), nth("li", 1), nth("tr", 0));
        // Permitido: un li junto a otro li; un párrafo tras el div; un div dentro de otro bloque.
        assert!(doc.resolve_drop(li0, li1, "after").is_ok());
        assert!(doc.resolve_drop(p, div, "after").is_ok());
        assert!(doc.resolve_drop(p, div, "inside").is_ok());
        assert!(doc.resolve_drop(td, tr, "inside").is_ok());
        // Rechazado, con el motivo.
        for (what, target, place) in [
            (li0, div, "inside"),  // <li> solo en listas
            (td, div, "inside"),   // <td> solo en filas
            (div, p, "inside"),    // un bloque no cabe en un <p>
            (p, ul, "inside"),     // una lista solo admite <li>
            (p, li0, "before"),    // ni como hermano: el padre es <ul>
            (div, div, "inside"),  // dentro de sí mismo
            (ul, li0, "inside"),   // dentro de un descendiente suyo
        ] {
            let r = doc.resolve_drop(what, target, place);
            assert!(r.is_err(), "{what} → {target} {place} debería rechazarse");
        }
        assert!(doc.resolve_drop(doc.find_tag("body").unwrap(), div, "after").is_err());
    }

    #[test]
    fn deleting_unclosed_item_stops_at_the_next_one() {
        let src = "<ul><li>A<li>B<li>C</ul>";
        let out = save(src, |d, h| {
            let id = id(d, "li");
            h.execute(d, Box::new(DeleteNode::new(id))).unwrap();
        });
        assert_eq!(out.mode, SaveMode::Preserved);
        assert_eq!(out.html, "<ul><li>B<li>C</ul>");
    }

    #[test]
    fn inserting_after_a_sibling_and_inside_an_empty_parent() {
        let src = "<ul><li>A</li><li>B</li></ul><div></div>";
        let out = save(src, |d, h| {
            let ul = id(d, "ul");
            h.execute(d, Box::new(InsertElement::new(ul, 1, "span".into()))).unwrap();
            let div = id(d, "div");
            h.execute(d, Box::new(InsertElement::new(div, 0, "hr".into()))).unwrap();
        });
        assert_eq!(out.mode, SaveMode::Preserved);
        assert_eq!(out.html, "<ul><li>A</li><span>Text</span><li>B</li></ul><div><hr></div>");
    }

    #[test]
    fn inserting_after_whitespace_text_goes_before_the_next_element() {
        let src = "<ul>\n  <li>A</li>\n  <li>B</li>\n</ul>";
        let out = save(src, |d, h| {
            let ul = id(d, "ul");
            // hijos: ws, li A, ws, li B, ws → índice 3 queda tras el ws previo a B.
            h.execute(d, Box::new(InsertElement::new(ul, 3, "hr".into()))).unwrap();
        });
        assert_eq!(out.mode, SaveMode::Preserved);
        assert_eq!(out.html, "<ul>\n  <li>A</li>\n  <hr>\n  <li>B</li>\n</ul>");
    }

    /// Inserta `tag` en el hijo número `index` de `parent_tag` y devuelve el texto guardado.
    fn insert_into(src: &str, parent_tag: &str, index: usize, tag: &str) -> SavedHtml {
        let out = save(src, |d, h| {
            let p = id(d, parent_tag);
            h.execute(d, Box::new(InsertElement::new(p, index, tag.into()))).unwrap();
        });
        assert_eq!(out.mode, SaveMode::Preserved, "{}", out.html);
        out
    }

    #[test]
    fn a_new_element_in_an_empty_body_gets_its_own_indented_line() {
        let blank = "<!DOCTYPE html>\n<html lang=\"es\">\n<head>\n<meta charset=\"utf-8\">\n<title></title>\n</head>\n<body>\n</body>\n</html>\n";
        let out = insert_into(blank, "body", 0, "div");
        assert_eq!(
            out.html,
            "<!DOCTYPE html>\n<html lang=\"es\">\n<head>\n<meta charset=\"utf-8\">\n<title></title>\n</head>\n<body>\n  <div>\n    <p>New block</p>\n  </div>\n</body>\n</html>\n"
        );
        assert!(!out.html.contains("\n\n") && !out.html.contains("\n  \n"));
    }

    #[test]
    fn a_new_element_at_the_end_follows_its_siblings_indent_and_leaves_no_blank_line() {
        let src = "<body>\n  <div>\n    <p>a</p>\n  </div>\n</body>";
        // Dentro del div, tras el párrafo (hijos: espacio, p, espacio).
        let out = insert_into(src, "div", 3, "hr");
        assert_eq!(out.html, "<body>\n  <div>\n    <p>a</p>\n    <hr>\n  </div>\n</body>");
        // Al final del cuerpo, tras el div.
        let out = insert_into(src, "body", 3, "hr");
        assert_eq!(out.html, "<body>\n  <div>\n    <p>a</p>\n  </div>\n  <hr>\n</body>");
        // Al principio de un contenedor con hijos.
        let out = insert_into(src, "div", 0, "hr");
        assert_eq!(out.html, "<body>\n  <div>\n    <hr>\n    <p>a</p>\n  </div>\n</body>");
    }

    #[test]
    fn the_file_decides_tabs_or_spaces_and_line_endings() {
        let tabs = "<body>\n\t<div>\n\t\t<p>a</p>\n\t</div>\n</body>";
        assert_eq!(insert_into(tabs, "body", 3, "hr").html, "<body>\n\t<div>\n\t\t<p>a</p>\n\t</div>\n\t<hr>\n</body>");
        let crlf = "<body>\r\n    <p>a</p>\r\n</body>";
        assert_eq!(insert_into(crlf, "body", 3, "hr").html, "<body>\r\n    <p>a</p>\r\n    <hr>\r\n</body>");
        // Sin hermanos con sangría: el padre más una unidad del archivo (4 espacios aquí).
        let wide = "<body>\n    <div>\n    </div>\n    <p>x</p>\n</body>";
        assert_eq!(
            insert_into(wide, "div", 0, "hr").html,
            "<body>\n    <div>\n        <hr>\n    </div>\n    <p>x</p>\n</body>"
        );
    }

    #[test]
    fn elements_with_children_are_spread_over_indented_lines() {
        let src = "<body>\n  <p>x</p>\n</body>";
        assert_eq!(
            insert_into(src, "body", 3, "ul").html,
            "<body>\n  <p>x</p>\n  <ul>\n    <li>Item</li>\n    <li>Item</li>\n  </ul>\n</body>"
        );
        let table = insert_into(src, "body", 3, "table").html;
        assert!(table.contains("\n  <table border=\"1\">\n    <tbody>\n      <tr>\n        <td>Cell</td>\n"), "{table}");
        assert!(!table.contains("\n\n") && !table.contains("\n  \n"), "{table}");
    }

    #[test]
    fn single_line_html_keeps_inserting_inline() {
        // Sin saltos de línea en el hueco no hay formato que respetar: se deja pegado.
        let out = insert_into("<body><p>x</p></body>", "body", 1, "hr");
        assert_eq!(out.html, "<body><p>x</p><hr></body>");
    }

    #[test]
    fn inline_elements_inside_text_are_not_split_onto_new_lines() {
        // Un <b> nuevo dentro de un párrafo con texto no puede añadir espacio visible.
        let out = insert_into("<body>\n  <p>\n    hola\n  </p>\n</body>", "p", 2, "b");
        assert!(!out.html.contains("\n    <b>"), "{}", out.html);
    }

    #[test]
    fn two_new_elements_in_a_row_each_get_a_line_in_order() {
        let src = "<body>\n</body>";
        let out = save(src, |d, h| {
            let b = id(d, "body");
            h.execute(d, Box::new(InsertElement::new(b, 0, "h1".into()))).unwrap();
            h.execute(d, Box::new(InsertElement::new(b, 1, "p".into()))).unwrap();
        });
        assert_eq!(out.mode, SaveMode::Preserved);
        assert_eq!(out.html, "<body>\n  <h1>Heading</h1>\n  <p>Text</p>\n</body>");
    }

    #[test]
    fn undo_restores_the_exact_original() {
        let out = save(MESSY, |d, h| {
            let id = id(d, "h1");
            h.execute(d, Box::new(SetAttr::new(id, "class".into(), Some("x".into())))).unwrap();
            h.undo(d);
        });
        assert_eq!(out.mode, SaveMode::Preserved);
        assert_eq!(out.html, MESSY);
    }

    #[test]
    fn code_edits_are_saved_literally_and_undo_walks_back_through_documents() {
        let first = "<p class=a>uno</p>";
        let typed = "<!DOCTYPE html>\n<style>@media (max-width:600px){ p{color:red} }</style>\n<P>dos</P>";
        let mut doc = Document::parse(first);
        let mut h = History::default();
        let p = id(&doc, "p");
        h.execute(&mut doc, Box::new(SetAttr::new(p, "class".into(), Some("b".into())))).unwrap();
        let patches = h.execute(&mut doc, Box::new(ReplaceDocument::new(typed))).unwrap();
        assert_eq!(patches, vec![Patch::Reload]);
        let saved = doc.to_saved_html();
        assert_eq!((saved.mode, saved.html.as_str()), (SaveMode::Preserved, typed));

        // Una edición visual posterior sigue funcionando sobre el documento nuevo.
        let p2 = id(&doc, "p");
        h.execute(&mut doc, Box::new(SetText::new(p2, "tres".into()))).unwrap();
        assert!(doc.to_saved_html().html.ends_with("<P>tres</P>"));

        h.undo(&mut doc);
        h.undo(&mut doc);
        assert_eq!(doc.to_saved_html().html, "<p class=\"b\">uno</p>");
        h.undo(&mut doc);
        assert_eq!(doc.to_saved_html().html, first);
        h.redo(&mut doc);
        h.redo(&mut doc);
        assert_eq!(doc.to_saved_html().html, typed);
    }

    #[test]
    fn inserted_image_carries_its_attributes_in_one_undo_step() {
        let src = "<body>\n  <p>x</p>\n</body>";
        let mut doc = Document::parse(src);
        let mut h = History::default();
        let body = id(&doc, "body");
        let attrs = vec![("src".to_string(), "img/a b.png".to_string()), ("alt".to_string(), "a b".to_string())];
        h.execute(&mut doc, Box::new(InsertElement::with_attrs(body, 1, "img".into(), attrs))).unwrap();
        let img = id(&doc, "img");
        assert_eq!(doc.attr(img, "src").as_deref(), Some("img/a b.png"));
        let saved = doc.to_saved_html();
        assert_eq!(saved.mode, SaveMode::Preserved);
        assert!(saved.html.contains(r#"<img src="img/a b.png" alt="a b">"#), "{}", saved.html);
        // Un solo Deshacer devuelve el original; rehacer conserva los atributos.
        h.undo(&mut doc);
        assert_eq!(doc.to_saved_html().html, src);
        h.redo(&mut doc);
        assert_eq!(doc.attr(id(&doc, "img"), "src").as_deref(), Some("img/a b.png"));
        // Un nombre de atributo peligroso se rechaza sin tocar el documento.
        let bad = vec![("onerror\"x".to_string(), "1".to_string())];
        assert!(h.execute(&mut doc, Box::new(InsertElement::with_attrs(body, 0, "img".into(), bad))).is_err());
    }

    #[test]
    fn unlocatable_target_falls_back_to_normalized() {
        // Documento vacío: <body> es implícito y no hay dónde anclar la inserción.
        let out = save("", |d, h| {
            let body = id(d, "body");
            h.execute(d, Box::new(InsertElement::new(body, 0, "hr".into()))).unwrap();
        });
        assert!(matches!(out.mode, SaveMode::Normalized(_)));
        assert!(out.html.contains("<hr>"));
    }

    #[test]
    fn rebase_makes_the_saved_text_the_new_baseline() {
        let mut doc = Document::parse("<p class=a>x</p><p>y</p>");
        let mut hist = History::default();
        let p = id(&doc, "p");
        hist.execute(&mut doc, Box::new(SetAttr::new(p, "class".into(), Some("b".into())))).unwrap();
        let first = doc.to_saved_html();
        assert_eq!(first.html, "<p class=\"b\">x</p><p>y</p>");
        doc.rebase_source(&first.html);
        hist.execute(&mut doc, Box::new(SetText::new(p, "z".into()))).unwrap();
        let second = doc.to_saved_html();
        assert_eq!(second.mode, SaveMode::Preserved);
        assert_eq!(second.html, "<p class=\"b\">z</p><p>y</p>");
    }

    #[test]
    fn hostile_inputs_never_panic_and_always_verify_or_fall_back() {
        let inputs = [
            "",
            "<",
            "<<<>>>",
            "<a href='x>texto</a>",
            "<b><i>mal</b>anidado</i>",
            "<table><tr><td>x<tr><td>y</table>",
            "<svg><path d='M0 0'/><circle r=1 /></svg><p>fin",
            "<!--c--><p>a<!--d-->b</p>",
            "<?php echo 1; ?><p>x</p>",
            "<textarea>\n\nx</textarea><pre>\ny</pre>",
            "<script>if (a<b) document.write('</p>')</script><p>ok",
            "\u{feff}<p>bom",
        ];
        for src in inputs {
            let out = save(src, |d, h| {
                if let Some(p) = d.find_tag("p") {
                    let _ = h.execute(d, Box::new(SetAttr::new(p, "data-x".into(), Some("1".into()))));
                    let _ = h.execute(d, Box::new(SetText::new(p, "nuevo".into())));
                }
                if let Some(b) = d.find_tag("body") {
                    let _ = h.execute(d, Box::new(InsertElement::new(b, 0, "div".into())));
                }
            });
            if out.mode == SaveMode::Preserved {
                // Lo que se da por bueno debe equivaler a lo editado al re-abrirlo.
                let again = Document::parse_plain(&out.html);
                assert!(!again.fingerprint().is_empty(), "{src:?}");
            }
        }
    }
}
