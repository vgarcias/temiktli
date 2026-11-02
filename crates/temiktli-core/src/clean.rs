//! Limpieza de HTML sucio (el que deja Word al copiar y pegar, sobre todo).
//!
//! Trabaja sobre el texto, no sobre el árbol: recorre las etiquetas del fragmento y solo
//! reescribe las que cambian, así que lo demás conserva su formato exacto. Las reglas son
//! propias; la lista de qué limpiar sale de lo que cualquier HTML pegado desde Word arrastra
//! (comentarios condicionales, `mso-*`, `<o:p>`, `style`, `lang`, spans vacíos…).

use crate::dom::Document;
use crate::i18n::t;
use crate::protocol::NodeId;

/// Qué se quitó o cambió, por familias.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CleanReport {
    /// Restos de Word: comentarios condicionales, `<o:p>` y similares, clases `Mso*`,
    /// marcadores `_Toc…`, `<style>` de Word.
    pub word: usize,
    /// Atributos `style`, `lang` y `xml:lang`, y los de `<font>`.
    pub attrs: usize,
    /// Etiquetas que no aportaban nada: `<span>` sin atributos, `<p>` vacíos, cierres y
    /// aperturas seguidos del mismo formato.
    pub redundant: usize,
    /// `<b>`→`<strong>`, `<i>`→`<em>` y spans con negrita/cursiva convertidos.
    pub semantic: usize,
}

impl CleanReport {
    pub fn total(&self) -> usize {
        self.word + self.attrs + self.redundant + self.semantic
    }

    /// Resumen para la barra de estado, sin las familias que no cambiaron.
    pub fn describe(&self) -> String {
        let parts: Vec<String> = [
            (self.word, "clean.word"),
            (self.attrs, "clean.attrs"),
            (self.redundant, "clean.redundant"),
            (self.semantic, "clean.semantic"),
        ]
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, key)| format!("{n} {}", t(key)))
        .collect();
        parts.join(&t("units.list_comma"))
    }
}

impl Document {
    /// El texto `html` con el interior del elemento `id` limpio. Solo cambia ese interior (el
    /// resto conserva su formato) y pasa por las mismas comprobaciones que `with_inner_html`.
    /// Si no hay nada que limpiar, devuelve `html` tal cual y un informe vacío.
    pub fn with_cleaned_inner(&self, html: &str, id: NodeId) -> Result<(String, CleanReport), String> {
        let index = self.source_index(html).ok_or_else(|| t("source.element_not_located"))?;
        let range = index.inner_range(id).ok_or_else(|| t("source.nothing_to_clean"))?;
        let inner = &html[range];
        let (cleaned, report) = clean_fragment(inner);
        if cleaned == inner {
            return Ok((html.to_string(), CleanReport::default()));
        }
        Ok((self.with_inner_html(html, id, &cleaned)?, report))
    }
}

// ---------------------------------------------------------------------------------------
// Lectura de etiquetas
// ---------------------------------------------------------------------------------------

/// Elementos cuyo contenido se deja intacto: texto crudo o con espacios que importan.
const VERBATIM: &[&str] = &["script", "style", "textarea", "title", "pre"];

struct Attr<'a> {
    /// Nombre en minúsculas.
    name: String,
    /// Valor sin comillas.
    value: Option<&'a str>,
    /// Texto original entero (`a="b"`).
    raw: &'a str,
}

struct OpenTag<'a> {
    raw: &'a str,
    name: &'a str,
    attrs: Vec<Attr<'a>>,
    self_closing: bool,
}

enum Tok<'a> {
    Text(&'a str),
    Comment(&'a str),
    /// Doctype, `<![if …]>`, `<?xml …?>`…
    Other(&'a str),
    Open(OpenTag<'a>),
    Close { name: &'a str, raw: &'a str },
}

fn parse_open(rest: &str) -> Option<(usize, OpenTag<'_>)> {
    let b = rest.as_bytes();
    let mut p = 1;
    while p < b.len() && !b[p].is_ascii_whitespace() && b[p] != b'/' && b[p] != b'>' {
        p += 1;
    }
    let name = &rest[1..p];
    let mut attrs = Vec::new();
    let mut self_closing = false;
    loop {
        while p < b.len() && b[p].is_ascii_whitespace() {
            p += 1;
        }
        match b.get(p)? {
            b'>' => {
                p += 1;
                break;
            }
            b'/' => {
                if b.get(p + 1) == Some(&b'>') {
                    self_closing = true;
                    p += 2;
                    break;
                }
                p += 1;
                continue;
            }
            _ => {}
        }
        let start = p;
        p += 1;
        while p < b.len() && !b[p].is_ascii_whitespace() && !matches!(b[p], b'=' | b'>' | b'/') {
            p += 1;
        }
        let name = rest[start..p].to_ascii_lowercase();
        let mut q = p;
        while q < b.len() && b[q].is_ascii_whitespace() {
            q += 1;
        }
        let mut value = None;
        if b.get(q) == Some(&b'=') {
            q += 1;
            while q < b.len() && b[q].is_ascii_whitespace() {
                q += 1;
            }
            match *b.get(q)? {
                quote @ (b'"' | b'\'') => {
                    let vs = q + 1;
                    let len = rest[vs..].find(quote as char)?;
                    value = Some(&rest[vs..vs + len]);
                    p = vs + len + 1;
                }
                _ => {
                    let vs = q;
                    while q < b.len() && !b[q].is_ascii_whitespace() && b[q] != b'>' {
                        q += 1;
                    }
                    value = Some(&rest[vs..q]);
                    p = q;
                }
            }
        }
        attrs.push(Attr { name, value, raw: &rest[start..p] });
    }
    Some((p, OpenTag { raw: &rest[..p], name, attrs, self_closing }))
}

fn tokenize(src: &str) -> Vec<Tok<'_>> {
    let low = src.to_ascii_lowercase();
    let b = src.as_bytes();
    let mut toks = Vec::new();
    let (mut i, mut text_from) = (0, 0);
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &src[i..];
        let next_alpha = b.get(i + 1).is_some_and(u8::is_ascii_alphabetic);
        let parsed: Option<(usize, Tok)> = if rest.starts_with("<!--") {
            Some(match rest[4..].find("-->") {
                Some(p) => (p + 7, Tok::Comment(&rest[..p + 7])),
                None => (rest.len(), Tok::Other(rest)),
            })
        } else if rest.starts_with("<!") || rest.starts_with("<?") {
            rest.find('>').map(|p| (p + 1, Tok::Other(&rest[..p + 1])))
        } else if rest.starts_with("</") && b.get(i + 2).is_some_and(u8::is_ascii_alphabetic) {
            rest.find('>').map(|p| {
                let name = rest[2..p].split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
                (p + 1, Tok::Close { name, raw: &rest[..p + 1] })
            })
        } else if next_alpha {
            parse_open(rest).map(|(len, o)| (len, Tok::Open(o)))
        } else {
            None
        };
        let Some((len, tok)) = parsed else {
            i += 1;
            continue;
        };
        if text_from < i {
            toks.push(Tok::Text(&src[text_from..i]));
        }
        i += len;
        let verbatim = match &tok {
            Tok::Open(o) if !o.self_closing => {
                let n = o.name.to_ascii_lowercase();
                VERBATIM.contains(&n.as_str()).then_some(n)
            }
            _ => None,
        };
        toks.push(tok);
        if let Some(n) = verbatim {
            let closer = format!("</{n}");
            let mut end = src.len();
            let mut from = i;
            while let Some(p) = low[from..].find(&closer) {
                let at = from + p;
                if !low.as_bytes().get(at + closer.len()).is_some_and(u8::is_ascii_alphanumeric) {
                    end = at;
                    break;
                }
                from = at + closer.len();
            }
            if i < end {
                toks.push(Tok::Text(&src[i..end]));
            }
            i = end;
        }
        text_from = i;
    }
    if text_from < src.len() {
        toks.push(Tok::Text(&src[text_from..]));
    }
    toks
}

// ---------------------------------------------------------------------------------------
// Limpieza
// ---------------------------------------------------------------------------------------

enum Out {
    Text(String),
    Other(String),
    /// `bare`: sin atributos.
    Open { name: String, raw: String, bare: bool },
    Close { name: String, raw: String },
}

/// Un elemento de los que se pueden quitar o convertir, abierto y por cerrar.
struct Entry {
    name: String,
    /// Qué emitir al cerrarlo, en orden: nombre y texto (`None`: el cierre original).
    closes: Vec<(String, Option<String>)>,
}

fn is_blank(t: &str) -> bool {
    t.replace("&nbsp;", "")
        .replace("&#160;", "")
        .replace("&#xa0;", "")
        .replace("&#xA0;", "")
        .replace('\u{a0}', "")
        .trim()
        .is_empty()
}

fn open_raw(name: &str, attrs: &[String], self_closing: bool) -> String {
    let mut s = format!("<{name}");
    for a in attrs {
        s.push(' ');
        s.push_str(a);
    }
    if self_closing {
        s.push_str(" /");
    }
    s.push('>');
    s
}

/// `(negrita, cursiva)` según las declaraciones de un atributo `style`.
fn style_flags(style: &str) -> (bool, bool) {
    let (mut bold, mut italic) = (false, false);
    for decl in style.split(';') {
        let Some((prop, val)) = decl.split_once(':') else { continue };
        let (prop, val) = (prop.trim().to_ascii_lowercase(), val.trim().to_ascii_lowercase());
        match prop.as_str() {
            "font-weight" => bold |= val == "bold" || val == "bolder" || val.parse::<u32>().is_ok_and(|w| w >= 600),
            "font-style" => italic |= val == "italic" || val == "oblique",
            _ => {}
        }
    }
    (bold, italic)
}

/// Atributos que se quedan, tal como estaban escritos (o reescritos si cambió su valor), y si
/// algo cambió. Quita `style`, `lang`/`xml:lang` y las clases `Mso*`.
fn keep_attrs(tag: &str, attrs: &[Attr], r: &mut CleanReport) -> (Vec<String>, bool) {
    let mut kept = Vec::new();
    let mut changed = false;
    for a in attrs {
        let font_junk = tag == "font" && matches!(a.name.as_str(), "face" | "size" | "color");
        match a.name.as_str() {
            "style" => {
                r.attrs += 1;
                changed = true;
            }
            "lang" | "xml:lang" if tag != "html" => {
                r.attrs += 1;
                changed = true;
            }
            _ if font_junk => {
                r.attrs += 1;
                changed = true;
            }
            "class" => {
                let all: Vec<&str> = a.value.unwrap_or("").split_whitespace().collect();
                let rest: Vec<&str> = all.iter().copied().filter(|c| !c.starts_with("Mso")).collect();
                if rest.len() == all.len() {
                    kept.push(a.raw.to_string());
                } else {
                    r.word += all.len() - rest.len();
                    changed = true;
                    if !rest.is_empty() {
                        kept.push(format!("class=\"{}\"", rest.join(" ")));
                    }
                }
            }
            _ => kept.push(a.raw.to_string()),
        }
    }
    (kept, changed)
}

fn attr_value<'a>(o: &'a OpenTag, name: &str) -> Option<&'a str> {
    o.attrs.iter().find(|a| a.name == name).and_then(|a| a.value)
}

fn is_word_comment(c: &str) -> bool {
    let low = c.to_ascii_lowercase();
    low.starts_with("<!--[if") || low.contains("[endif]")
}

fn is_word_other(t: &str) -> bool {
    let low = t.to_ascii_lowercase();
    low.starts_with("<![if") || low.starts_with("<![endif") || low.starts_with("<?xml")
}

fn is_word_style(css: &str) -> bool {
    let low = css.to_ascii_lowercase();
    low.contains("mso-") || low.contains("msonormal") || low.contains("@font-face")
}

/// Limpia un fragmento de HTML. Todas las reglas están activas.
pub fn clean_fragment(src: &str) -> (String, CleanReport) {
    let mut r = CleanReport::default();
    let toks = tokenize(src);
    let mut out: Vec<Out> = Vec::with_capacity(toks.len());
    let mut stack: Vec<Entry> = Vec::new();

    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            Tok::Text(t) => out.push(Out::Text((*t).to_string())),
            Tok::Comment(c) => {
                if is_word_comment(c) {
                    r.word += 1;
                } else {
                    out.push(Out::Other((*c).to_string()));
                }
            }
            Tok::Other(t) => {
                if is_word_other(t) {
                    r.word += 1;
                } else {
                    out.push(Out::Other((*t).to_string()));
                }
            }
            Tok::Close { name, raw } => {
                let lname = name.to_ascii_lowercase();
                let managed = matches!(lname.as_str(), "span" | "font" | "b" | "i" | "a") || lname.contains(':');
                match stack.iter().rposition(|e| e.name == lname).filter(|_| managed) {
                    Some(pos) => {
                        // Lo que quedó abierto por dentro también se cierra, de dentro a fuera.
                        while stack.len() > pos {
                            let e = stack.pop().expect("hay al menos `pos` + 1 elementos");
                            for (n, text) in e.closes {
                                out.push(Out::Close { name: n, raw: text.unwrap_or_else(|| (*raw).to_string()) });
                            }
                        }
                    }
                    None => out.push(Out::Close { name: lname, raw: (*raw).to_string() }),
                }
            }
            Tok::Open(o) => {
                let lname = o.name.to_ascii_lowercase();
                // Restos de Word que se quitan enteros.
                if lname.contains(':') {
                    r.word += 1;
                    if !o.self_closing {
                        stack.push(Entry { name: lname, closes: Vec::new() });
                    }
                    i += 1;
                    continue;
                }
                if lname == "meta"
                    && matches!(
                        attr_value(o, "name").map(str::to_ascii_lowercase).as_deref(),
                        Some("generator" | "progid" | "originator")
                    )
                {
                    r.word += 1;
                    i += 1;
                    continue;
                }
                if lname == "link"
                    && matches!(
                        attr_value(o, "rel").map(str::to_ascii_lowercase).as_deref(),
                        Some("file-list" | "edit-time-data" | "themedata" | "colorschememapping")
                    )
                {
                    r.word += 1;
                    i += 1;
                    continue;
                }
                if lname == "style" {
                    if let (Some(Tok::Text(css)), Some(Tok::Close { name, .. })) = (toks.get(i + 1), toks.get(i + 2)) {
                        if name.eq_ignore_ascii_case("style") && is_word_style(css) {
                            r.word += 1;
                            i += 3;
                            continue;
                        }
                    }
                }
                let style = o.attrs.iter().find(|a| a.name == "style").and_then(|a| a.value).unwrap_or("");
                let (bold, italic) = style_flags(style);
                let (kept, changed) = keep_attrs(&lname, &o.attrs, &mut r);

                match lname.as_str() {
                    "span" | "font" if !o.self_closing => {
                        let mut closes: Vec<(String, Option<String>)> = Vec::new();
                        if kept.is_empty() {
                            r.redundant += 1;
                        } else {
                            let raw = if changed { open_raw(o.name, &kept, false) } else { o.raw.to_string() };
                            out.push(Out::Open { name: lname.clone(), raw, bare: false });
                            closes.push((lname.clone(), None));
                        }
                        for (on, tag) in [(bold, "strong"), (italic, "em")] {
                            if on {
                                r.semantic += 1;
                                out.push(Out::Open { name: tag.into(), raw: format!("<{tag}>"), bare: true });
                                closes.insert(0, (tag.into(), Some(format!("</{tag}>"))));
                            }
                        }
                        stack.push(Entry { name: lname, closes });
                    }
                    "b" | "i" if !o.self_closing => {
                        let tag = if lname == "b" { "strong" } else { "em" };
                        r.semantic += 1;
                        out.push(Out::Open { name: tag.into(), raw: open_raw(tag, &kept, false), bare: kept.is_empty() });
                        stack.push(Entry { name: lname, closes: vec![(tag.into(), Some(format!("</{tag}>")))] });
                    }
                    "a" if !o.self_closing => {
                        let bookmark = attr_value(o, "name").is_some_and(|n| n.starts_with('_'))
                            && !o.attrs.iter().any(|a| a.name == "href")
                            && o.attrs.iter().all(|a| matches!(a.name.as_str(), "name" | "style" | "lang"));
                        if bookmark {
                            r.word += 1;
                            stack.push(Entry { name: lname, closes: Vec::new() });
                        } else {
                            let raw = if changed { open_raw(o.name, &kept, false) } else { o.raw.to_string() };
                            out.push(Out::Open { name: lname.clone(), raw, bare: kept.is_empty() });
                            stack.push(Entry { name: lname, closes: vec![("a".into(), None)] });
                        }
                    }
                    _ => {
                        let raw = if changed { open_raw(o.name, &kept, o.self_closing) } else { o.raw.to_string() };
                        out.push(Out::Open { name: lname, raw, bare: kept.is_empty() });
                    }
                }
            }
        }
        i += 1;
    }

    simplify(&mut out, &mut r);

    let mut result = String::with_capacity(src.len());
    for o in &out {
        match o {
            Out::Text(s) | Out::Other(s) => result.push_str(s),
            Out::Open { raw, .. } | Out::Close { raw, .. } => result.push_str(raw),
        }
    }
    (result, r)
}

/// Elementos que se quitan si no tienen atributos ni contenido.
const EMPTYABLE: &[&str] = &["span", "b", "strong", "i", "em", "u", "font", "p"];
/// Formatos de texto: dos seguidos iguales (`</b><b>`) son uno.
const MERGEABLE: &[&str] = &["b", "strong", "i", "em", "u"];

/// ¿La apertura que corresponde al cierre `out[close]` no tiene atributos?
fn opener_is_bare(out: &[Out], close: usize, name: &str) -> bool {
    let mut depth = 0;
    for tok in out[..close].iter().rev() {
        match tok {
            Out::Close { name: n, .. } if n == name => depth += 1,
            Out::Open { name: n, bare, .. } if n == name => {
                if depth == 0 {
                    return *bare;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    false
}

fn blank_text(tok: &Out) -> bool {
    matches!(tok, Out::Text(s) if is_blank(s))
}

/// Quita etiquetas vacías y junta formatos contiguos, hasta que no quede nada por hacer.
fn simplify(out: &mut Vec<Out>, r: &mut CleanReport) {
    loop {
        let mut changed = false;
        let mut i = 0;
        while i < out.len() {
            // Apertura sin atributos seguida de su cierre con solo espacios dentro.
            if let Out::Open { name, bare: true, .. } = &out[i] {
                if EMPTYABLE.contains(&name.as_str()) {
                    let mut j = i + 1;
                    while j < out.len() && matches!(out[j], Out::Text(_)) {
                        j += 1;
                    }
                    let closes_here = matches!(out.get(j), Some(Out::Close { name: n, .. }) if n == name);
                    if closes_here && out[i + 1..j].iter().all(blank_text) {
                        if name == "p" {
                            out.drain(i..=j);
                        } else {
                            // Un espacio entre palabras tiene que quedarse.
                            out.remove(j);
                            out.remove(i);
                        }
                        r.redundant += 1;
                        changed = true;
                        continue;
                    }
                }
            }
            // `</b>` seguido (quizá tras un espacio) de `<b>`: es la misma negrita.
            if let Out::Close { name, .. } = &out[i] {
                if MERGEABLE.contains(&name.as_str()) {
                    let mut j = i + 1;
                    while j < out.len() && blank_text(&out[j]) {
                        j += 1;
                    }
                    let reopens = matches!(out.get(j), Some(Out::Open { name: n, bare: true, .. }) if n == name);
                    if reopens && opener_is_bare(out, i, name) {
                        out.remove(j);
                        out.remove(i);
                        r.redundant += 1;
                        changed = true;
                        continue;
                    }
                }
            }
            i += 1;
        }
        if !changed {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(s: &str) -> String {
        clean_fragment(s).0
    }

    #[test]
    fn word_paragraph_becomes_plain_html() {
        let src = "<p class=MsoNormal style='margin:0cm;mso-pagination:widow-orphan'><span lang=ES style='font-size:12.0pt'>Hola <b>mundo</b></span><o:p></o:p></p>";
        let (out, report) = clean_fragment(src);
        assert_eq!(out, "<p>Hola <strong>mundo</strong></p>");
        assert_eq!(report.word, 2); // la clase Mso y <o:p>
        assert_eq!(report.attrs, 3); // style y lang del span, style del p
        assert_eq!(report.semantic, 1);
        assert_eq!(report.redundant, 1); // el span
        assert!(report.describe().contains("Word leftovers"));
    }

    #[test]
    fn conditional_comments_and_word_markers_go_but_user_comments_stay() {
        let src = "<!--[if gte mso 9]><xml><o:OfficeDocumentSettings/></xml><![endif]--><!-- mi nota --><![if !supportLists]>x<![endif]><p>a</p>";
        assert_eq!(clean(src), "<!-- mi nota -->x<p>a</p>");
    }

    #[test]
    fn word_style_blocks_go_but_the_users_stay() {
        assert_eq!(clean("<style>p.MsoNormal{mso-style-parent:\"\";margin:0}</style><p>a</p>"), "<p>a</p>");
        let mine = "<style>.card{color:red}</style><p>a</p>";
        assert_eq!(clean(mine), mine);
    }

    #[test]
    fn spans_with_bold_or_italic_become_semantic_tags() {
        assert_eq!(clean("<p><span style='font-weight:bold'>fuerte</span></p>"), "<p><strong>fuerte</strong></p>");
        assert_eq!(clean("<p><span style=\"font-style:italic\">suave</span></p>"), "<p><em>suave</em></p>");
        assert_eq!(
            clean("<p><span style='font-weight:700;font-style:italic'>ambos</span></p>"),
            "<p><strong><em>ambos</em></strong></p>"
        );
        // Con otros atributos el span se queda y lleva la etiqueta semántica dentro.
        assert_eq!(
            clean("<p><span id=x style='font-weight:bold'>t</span></p>"),
            "<p><span id=x><strong>t</strong></span></p>"
        );
        // Un peso normal no es negrita.
        assert_eq!(clean("<p><span style='font-weight:400'>t</span></p>"), "<p>t</p>");
    }

    #[test]
    fn b_and_i_become_strong_and_em_keeping_their_other_attributes() {
        assert_eq!(clean("<b>a</b> <i class=\"x\">b</i>"), "<strong>a</strong> <em class=\"x\">b</em>");
        assert_eq!(clean("<B>a</B>"), "<strong>a</strong>");
    }

    #[test]
    fn empty_tags_and_repeated_formats_are_removed() {
        assert_eq!(clean("<p>a</p><p>&nbsp;</p><p> </p><p>b</p>"), "<p>a</p><p>b</p>");
        // Un espacio entre palabras no se pierde al quitar el formato vacío.
        assert_eq!(clean("a<b> </b>b"), "a b");
        assert_eq!(clean("<b>uno</b><b>dos</b>"), "<strong>unodos</strong>");
        assert_eq!(clean("<b>uno</b> <b>dos</b>"), "<strong>uno dos</strong>");
        // Cascada: al quitar lo de dentro, lo de fuera queda vacío.
        assert_eq!(clean("<p><span><b></b></span></p><p>x</p>"), "<p>x</p>");
        // Con atributos, no es redundante.
        assert_eq!(clean("<p class=\"gap\"></p>"), "<p class=\"gap\"></p>");
        assert_eq!(clean("<b class=\"a\">uno</b><b>dos</b>"), "<strong class=\"a\">uno</strong><strong>dos</strong>");
    }

    #[test]
    fn only_word_classes_are_removed_and_the_rest_are_kept() {
        assert_eq!(clean("<p class=\"hero MsoBodyText big\">a</p>"), "<p class=\"hero big\">a</p>");
        assert_eq!(clean("<p class=\"MsoNormal\" id=\"p1\">a</p>"), "<p id=\"p1\">a</p>");
        assert_eq!(clean("<p class=\"hero\">a</p>"), "<p class=\"hero\">a</p>");
    }

    #[test]
    fn untouched_tags_keep_their_exact_text() {
        let src = "<div  id = 'a'   data-x=1>\n  <img src=a.png alt=''>\n  <a href=\"#x\" title='t'>enlace</a>\n</div>";
        assert_eq!(clean(src), src);
        assert_eq!(clean_fragment(src).1.total(), 0);
    }

    #[test]
    fn scripts_and_preformatted_text_are_left_alone() {
        let src = "<script>var a = '<b>'; /* <span style=x> */</script><pre><b>x</b>  <span style='a'>y</span></pre>";
        assert_eq!(clean(src), src);
    }

    #[test]
    fn word_bookmarks_and_namespaced_tags_are_unwrapped_but_real_links_stay() {
        assert_eq!(clean("<h1><a name=\"_Toc123\"></a>Título</h1>"), "<h1>Título</h1>");
        assert_eq!(clean("<p><st1:place>Cancún</st1:place></p>"), "<p>Cancún</p>");
        assert_eq!(clean("<a name=\"top\">ancla</a>"), "<a name=\"top\">ancla</a>");
        assert_eq!(clean("<a href=\"https://x.mx\" style='color:blue'>x</a>"), "<a href=\"https://x.mx\">x</a>");
    }

    #[test]
    fn font_tags_lose_their_presentation_attributes() {
        assert_eq!(clean("<p><font face=\"Arial\" size=3 color=red>texto</font></p>"), "<p>texto</p>");
    }

    #[test]
    fn cleaning_twice_changes_nothing_more() {
        let src = "<p class=MsoNormal><span style='mso-bidi-font-weight:bold'><b>A</b><b>B</b></span><o:p>&nbsp;</o:p></p><p>&nbsp;</p>";
        let once = clean(src);
        let (twice, report) = clean_fragment(&once);
        assert_eq!(once, twice);
        assert_eq!(report.total(), 0);
    }

    #[test]
    fn visible_text_is_never_lost() {
        let src = "<p class=MsoNormal style='a:b'><span lang=ES>Hola</span> <b>don</b><i> </i>Pepe<o:p>&nbsp;</o:p></p>";
        let strip = |s: &str| {
            let mut t = String::new();
            let mut in_tag = false;
            for c in s.chars() {
                match c {
                    '<' => in_tag = true,
                    '>' => in_tag = false,
                    _ if !in_tag => t.push(c),
                    _ => {}
                }
            }
            t.replace("&nbsp;", "").split_whitespace().collect::<String>()
        };
        assert_eq!(strip(&clean(src)), strip(src));
    }

    #[test]
    fn unbalanced_input_does_not_panic() {
        for src in ["<span><b>x", "x</span></b>", "<p class='a", "<", "<!--", "<a name=_x>", "<font><b>", "</o:p>"] {
            let _ = clean_fragment(src);
        }
    }

    #[test]
    fn cleaning_a_document_changes_only_the_body_and_keeps_its_format() {
        let html = "<!DOCTYPE html>\n<html>\n<head>\n  <title>T</title>\n</head>\n<body>\n  <p class=MsoNormal style='x:y'><span lang=ES>Hola</span></p>\n  <div  id='k'>fin</div>\n</body>\n</html>";
        let d = Document::parse(html);
        let saved = d.to_saved_html().html;
        let body = d.find_tag("body").unwrap();
        let (new, report) = d.with_cleaned_inner(&saved, body).unwrap();
        assert_eq!(
            new,
            "<!DOCTYPE html>\n<html>\n<head>\n  <title>T</title>\n</head>\n<body>\n  <p>Hola</p>\n  <div  id='k'>fin</div>\n</body>\n</html>"
        );
        assert!(report.total() > 0);
        // Ya limpio: nada que hacer.
        let again = Document::parse(&new);
        let (same, report) = again.with_cleaned_inner(&new, again.find_tag("body").unwrap()).unwrap();
        assert_eq!(same, new);
        assert_eq!(report.total(), 0);
    }

    #[test]
    fn cleaning_a_selected_element_leaves_the_rest_of_the_page_alone() {
        let html = "<!DOCTYPE html><html><body><div id=\"a\"><p style=\"x:y\">uno</p></div><p style=\"x:y\">dos</p></body></html>";
        let d = Document::parse(html);
        let saved = d.to_saved_html().html;
        let div = (1..50).find(|&i| d.tag(i).as_deref() == Some("div")).unwrap();
        let (new, _) = d.with_cleaned_inner(&saved, div).unwrap();
        assert!(new.contains("<div id=\"a\"><p>uno</p></div><p style=\"x:y\">dos</p>"), "{new}");
    }
}
