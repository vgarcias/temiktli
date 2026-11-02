//! Análisis del HTML original con la posición exacta (en bytes) de cada etiqueta.
//!
//! html5ever no expone posiciones, así que alimentamos su tokenizador carácter a
//! carácter y anotamos cuántos bytes se habían entregado cuando salió cada token de
//! etiqueta. Ese final es fiable; el inicio se deduce examinando el texto hacia atrás
//! (ver `locate`). Todo rango se valida contra el texto: si algo no cuadra, el
//! análisis completo devuelve `None` y el llamador guarda de forma normalizada.

use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{
    BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use html5ever::tree_builder::{
    ElementFlags, NodeOrText, QuirksMode, TreeBuilder, TreeBuilderOpts, TreeSink,
};
use html5ever::{Attribute, ExpandedName, QualName, ns};
use markup5ever::TokenizerResult;
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

/// Cuánto retrocedemos como máximo buscando el `<` de una etiqueta.
const MAX_TAG_LEN: usize = 64 * 1024;

/// Etiqueta localizada en el texto, con sus atributos en bruto.
#[derive(Debug, Clone)]
pub(crate) struct TagScan {
    pub start: usize,
    pub end: usize,
    pub is_end: bool,
    /// Nombre de atributo en minúsculas y rango de su texto original (`a="b"`).
    pub attrs: Vec<(String, Range<usize>)>,
    pub self_closing: bool,
    /// Dónde insertar atributos nuevos: el `/` de `/>` o el `>` final.
    pub close_at: usize,
    name: Range<usize>,
}

#[derive(Debug)]
pub(crate) struct TagTok {
    pub name: String,
    pub scan: TagScan,
    /// Índice (en preorden) del elemento que creó esta etiqueta de inicio.
    pub element: Option<usize>,
}

#[derive(Debug)]
pub(crate) struct ParsedElem {
    pub name: String,
    pub parent: Option<usize>,
    /// Fuera del espacio de nombres HTML (SVG, MathML): `<x/>` sí cierra.
    pub foreign: bool,
}

#[derive(Debug)]
pub(crate) struct Spans {
    /// Elementos del árbol en preorden, igual que el orden de asignación de ids.
    pub elems: Vec<ParsedElem>,
    /// Etiquetas de inicio y fin localizadas, en orden de aparición.
    pub tags: Vec<TagTok>,
}

pub(crate) fn parse_spans(src: &str) -> Option<Spans> {
    let st = Rc::new(State::default());
    let sink = SpanDom { dom: RcDom::default(), st: st.clone() };
    let tb = TreeBuilder::new(sink, TreeBuilderOpts::default());
    let tok = Tokenizer::new(SpanSink { inner: tb, st: st.clone() }, TokenizerOpts::default());
    let queue = BufferQueue::default();
    for ch in src.chars() {
        st.pos.set(st.pos.get() + ch.len_utf8());
        queue.push_back(StrTendril::from(ch.to_string()));
        while !matches!(tok.feed(&queue), TokenizerResult::Done) {}
    }
    tok.end();
    let dom = tok.sink.inner.sink.finish();

    let mut elems = Vec::new();
    let mut keys = HashMap::new();
    collect(&dom.document, None, &mut elems, &mut keys);

    let raw = st.tags.take();
    let bytes = src.as_bytes();
    let mut tags = Vec::new();
    let mut prev_end = 0;
    for r in raw {
        let element = r.element.and_then(|k| keys.get(&k).copied());
        match locate(bytes, prev_end, r.end, r.is_end, &r.name) {
            Some(scan) => {
                prev_end = scan.end;
                tags.push(TagTok { name: r.name, scan, element });
            }
            // Una etiqueta sin efecto en el árbol se puede ignorar; cualquier otra no.
            None if element.is_none() && !r.is_end => {}
            None => return None,
        }
    }
    Some(Spans { elems, tags })
}

fn collect(
    h: &Handle,
    parent: Option<usize>,
    elems: &mut Vec<ParsedElem>,
    keys: &mut HashMap<usize, usize>,
) {
    let mut me = parent;
    if let NodeData::Element { name, .. } = &h.data {
        let i = elems.len();
        keys.insert(key(h), i);
        elems.push(ParsedElem {
            name: name.local.to_string(),
            parent,
            foreign: name.ns != ns!(html),
        });
        me = Some(i);
    }
    for c in h.children.borrow().iter() {
        collect(c, me, elems, keys);
    }
}

// ---------- Localización y validación de una etiqueta ----------

/// Encuentra la etiqueta que termina en `end` (o un byte después: ver abajo) y cuyo
/// nombre coincide. Si varios `<` encajan, gana el más tardío, el más específico.
fn locate(src: &[u8], lower: usize, end: usize, is_end: bool, name: &str) -> Option<TagScan> {
    // El tokenizador puede avisar un byte antes de lo debido cuando retiene un `\r`.
    let end = if end > 0 && src.get(end - 1) == Some(&b'>') {
        end
    } else if src.get(end) == Some(&b'>') {
        end + 1
    } else {
        return None;
    };
    let floor = lower.max(end.saturating_sub(MAX_TAG_LEN));
    (floor..end).rev().filter(|&p| src[p] == b'<').find_map(|p| {
        let s = scan_tag(src, p)?;
        let same_name = src[s.name.clone()].eq_ignore_ascii_case(name.as_bytes());
        (s.end == end && s.is_end == is_end && same_name).then_some(s)
    })
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
}

/// Lee una etiqueta que empieza en `p` (un `<`) siguiendo las reglas de comillas del
/// tokenizador HTML, sin decodificar nada. `None` si no es una etiqueta completa.
pub(crate) fn scan_tag(b: &[u8], p: usize) -> Option<TagScan> {
    let n = b.len();
    let mut i = p + 1;
    let is_end = b.get(i) == Some(&b'/');
    if is_end {
        i += 1;
    }
    if !b.get(i)?.is_ascii_alphabetic() {
        return None;
    }
    let name_start = i;
    while i < n && !is_ws(b[i]) && b[i] != b'/' && b[i] != b'>' {
        i += 1;
    }
    let name = name_start..i;
    let mut attrs = Vec::new();
    loop {
        while i < n && is_ws(b[i]) {
            i += 1;
        }
        match *b.get(i)? {
            b'>' => {
                return Some(TagScan {
                    start: p,
                    end: i + 1,
                    is_end,
                    attrs,
                    self_closing: false,
                    close_at: i,
                    name,
                });
            }
            b'/' if b.get(i + 1) == Some(&b'>') => {
                return Some(TagScan {
                    start: p,
                    end: i + 2,
                    is_end,
                    attrs,
                    self_closing: true,
                    close_at: i,
                    name,
                });
            }
            b'/' => i += 1,
            _ => {
                let a_start = i;
                // El primer carácter siempre pertenece al nombre (incluso un `=`).
                i += 1;
                while i < n && !is_ws(b[i]) && !matches!(b[i], b'/' | b'>' | b'=') {
                    i += 1;
                }
                let a_name = String::from_utf8_lossy(&b[a_start..i]).to_ascii_lowercase();
                let after_name = i;
                while i < n && is_ws(b[i]) {
                    i += 1;
                }
                if b.get(i) == Some(&b'=') {
                    i += 1;
                    while i < n && is_ws(b[i]) {
                        i += 1;
                    }
                    match *b.get(i)? {
                        q @ (b'"' | b'\'') => {
                            let close = b[i + 1..].iter().position(|&c| c == q)?;
                            i += close + 2;
                        }
                        _ => {
                            while i < n && !is_ws(b[i]) && b[i] != b'>' {
                                i += 1;
                            }
                        }
                    }
                } else {
                    i = after_name;
                }
                attrs.push((a_name, a_start..i));
            }
        }
    }
}

// ---------- Anotación del tokenizador y del constructor de árbol ----------

struct RawTag {
    is_end: bool,
    name: String,
    end: usize,
    /// Identidad (puntero) del elemento que creó esta etiqueta.
    element: Option<usize>,
}

#[derive(Default)]
struct State {
    /// Bytes entregados al tokenizador hasta ahora.
    pos: Cell<usize>,
    /// Etiqueta que se está procesando, si el token actual es una etiqueta.
    current: Cell<Option<usize>>,
    tags: RefCell<Vec<RawTag>>,
}

fn key(h: &Handle) -> usize {
    Rc::as_ptr(h) as usize
}

struct SpanSink<S> {
    inner: S,
    st: Rc<State>,
}

impl<S: TokenSink> TokenSink for SpanSink<S> {
    type Handle = S::Handle;

    fn process_token(&self, token: Token, line: u64) -> TokenSinkResult<Self::Handle> {
        match &token {
            Token::TagToken(t) => {
                let mut tags = self.st.tags.borrow_mut();
                tags.push(RawTag {
                    is_end: t.kind == TagKind::EndTag,
                    name: t.name.to_string(),
                    end: self.st.pos.get(),
                    element: None,
                });
                self.st.current.set(Some(tags.len() - 1));
            }
            _ => self.st.current.set(None),
        }
        let r = self.inner.process_token(token, line);
        self.st.current.set(None);
        r
    }

    fn end(&self) {
        self.inner.end()
    }

    fn adjusted_current_node_present_but_not_in_html_namespace(&self) -> bool {
        self.inner.adjusted_current_node_present_but_not_in_html_namespace()
    }
}

/// `RcDom` que además enlaza cada elemento con la etiqueta de inicio que lo creó.
struct SpanDom {
    dom: RcDom,
    st: Rc<State>,
}

impl TreeSink for SpanDom {
    type Handle = Handle;
    type Output = RcDom;
    type ElemName<'a> = ExpandedName<'a>;

    fn finish(self) -> RcDom {
        self.dom.finish()
    }
    fn parse_error(&self, msg: Cow<'static, str>) {
        self.dom.parse_error(msg)
    }
    fn get_document(&self) -> Handle {
        self.dom.get_document()
    }
    fn elem_name<'a>(&'a self, target: &'a Handle) -> ExpandedName<'a> {
        self.dom.elem_name(target)
    }
    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> Handle {
        let local = name.local.to_string();
        let h = self.dom.create_element(name, attrs, flags);
        if let Some(i) = self.st.current.get() {
            let mut tags = self.st.tags.borrow_mut();
            let t = &mut tags[i];
            // Solo la primera creación de una etiqueta de inicio es la suya; las demás
            // (reconstrucción de formato, elementos implícitos) no tienen texto propio.
            if !t.is_end && t.element.is_none() && t.name.eq_ignore_ascii_case(&local) {
                t.element = Some(key(&h));
            }
        }
        h
    }
    fn create_comment(&self, text: StrTendril) -> Handle {
        self.dom.create_comment(text)
    }
    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Handle {
        self.dom.create_pi(target, data)
    }
    fn append(&self, parent: &Handle, child: NodeOrText<Handle>) {
        self.dom.append(parent, child)
    }
    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        prev_element: &Handle,
        child: NodeOrText<Handle>,
    ) {
        self.dom.append_based_on_parent_node(element, prev_element, child)
    }
    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        self.dom.append_doctype_to_document(name, public_id, system_id)
    }
    fn mark_script_already_started(&self, node: &Handle) {
        self.dom.mark_script_already_started(node)
    }
    fn pop(&self, node: &Handle) {
        self.dom.pop(node)
    }
    fn get_template_contents(&self, target: &Handle) -> Handle {
        self.dom.get_template_contents(target)
    }
    fn same_node(&self, x: &Handle, y: &Handle) -> bool {
        self.dom.same_node(x, y)
    }
    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.dom.set_quirks_mode(mode)
    }
    fn append_before_sibling(&self, sibling: &Handle, new_node: NodeOrText<Handle>) {
        self.dom.append_before_sibling(sibling, new_node)
    }
    fn add_attrs_if_missing(&self, target: &Handle, attrs: Vec<Attribute>) {
        self.dom.add_attrs_if_missing(target, attrs)
    }
    fn associate_with_form(&self, target: &Handle, form: &Handle, nodes: (&Handle, Option<&Handle>)) {
        self.dom.associate_with_form(target, form, nodes)
    }
    fn remove_from_parent(&self, target: &Handle) {
        self.dom.remove_from_parent(target)
    }
    fn reparent_children(&self, node: &Handle, new_parent: &Handle) {
        self.dom.reparent_children(node, new_parent)
    }
    fn is_mathml_annotation_xml_integration_point(&self, h: &Handle) -> bool {
        self.dom.is_mathml_annotation_xml_integration_point(h)
    }
    fn set_current_line(&self, n: u64) {
        self.dom.set_current_line(n)
    }
    fn allow_declarative_shadow_roots(&self, p: &Handle) -> bool {
        self.dom.allow_declarative_shadow_roots(p)
    }
    fn attach_declarative_shadow(&self, l: &Handle, t: &Handle, a: &[Attribute]) -> bool {
        self.dom.attach_declarative_shadow(l, t, a)
    }
    fn maybe_clone_an_option_into_selectedcontent(&self, o: &Handle) {
        self.dom.maybe_clone_an_option_into_selectedcontent(o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags_of(src: &str) -> Vec<String> {
        let s = parse_spans(src).expect("debe localizar todas las etiquetas");
        s.tags.iter().map(|t| src[t.scan.start..t.scan.end].to_string()).collect()
    }

    #[test]
    fn locates_tags_with_quirky_syntax() {
        let src = "<HTML lang=es><body class='a'><a title=\"a>b<c\" href=x>e</a><br/></BODY></HTML>";
        assert_eq!(
            tags_of(src),
            [
                "<HTML lang=es>",
                "<body class='a'>",
                "<a title=\"a>b<c\" href=x>",
                "</a>",
                "<br/>",
                "</BODY>",
                "</HTML>"
            ]
        );
    }

    #[test]
    fn handles_crlf_script_and_entities() {
        let src = "<body>\r\n<script>if (a<b) { x = '</div>'; }</script>\r\n<p>&x<b>neg</b></p></body>";
        let t = tags_of(src);
        assert!(t.contains(&"<script>".to_string()));
        assert!(t.contains(&"</script>".to_string()));
        assert!(t.contains(&"<b>".to_string()));
        assert!(!t.contains(&"</div>".to_string()));
    }

    #[test]
    fn utf8_offsets_are_bytes() {
        let src = "<p>Ñandú ©</p><div>ok</div>";
        let s = parse_spans(src).unwrap();
        let div = s.tags.iter().find(|t| t.name == "div").unwrap();
        assert_eq!(&src[div.scan.start..div.scan.end], "<div>");
    }

    #[test]
    fn links_elements_in_preorder() {
        let src = "<p>a</p><p>b</p>";
        let s = parse_spans(src).unwrap();
        let names: Vec<_> = s.elems.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["html", "head", "body", "p", "p"]);
        // html, head y body son implícitos: ninguna etiqueta los creó.
        let linked: Vec<_> = s.tags.iter().filter_map(|t| t.element).collect();
        assert_eq!(linked, [3, 4]);
    }

    #[test]
    fn scan_records_attribute_ranges_and_close_point() {
        let src = b"<img src=a ALT='x y' data-k disabled />";
        let s = scan_tag(src, 0).unwrap();
        let names: Vec<_> = s.attrs.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["src", "alt", "data-k", "disabled"]);
        assert_eq!(&src[s.attrs[1].1.clone()], b"ALT='x y'");
        assert!(s.self_closing);
        assert_eq!(src[s.close_at], b'/');
    }

    #[test]
    fn incomplete_or_foreign_text_is_not_a_tag() {
        assert!(scan_tag(b"<a href=\"x", 0).is_none());
        assert!(scan_tag(b"< p>", 0).is_none());
        assert!(scan_tag(b"<3 >", 0).is_none());
    }

    #[test]
    fn never_panics_on_garbage() {
        for src in ["", "<", "<<<>>>", "<a", "<a b='", "</", "<!--", "\u{feff}<p>", "<p\r\n>x</p\r\n>"] {
            let _ = parse_spans(src);
        }
    }
}
