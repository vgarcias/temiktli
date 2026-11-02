//! Formato del HTML que se inserta en el texto del archivo: cada elemento nuevo va en su
//! propia línea, con la sangría de sus hermanos (o la del padre más un nivel), y los que
//! llevan hijos (`ul`, `table`, `select`…) se reparten en varias líneas. La unidad de
//! sangría y los saltos de línea se toman del propio archivo.

use crate::dom::is_void;
use crate::tags::{is_phrasing_tag, is_text_only_tag};

/// Contenido que no se toca nunca: el espacio dentro de estas etiquetas es texto.
const RAW: &[&str] = &["script", "style", "textarea", "pre", "title", "listing"];

/// Padres donde el espacio entre hijos no se ve: se puede partir en líneas sin cambiar la página.
pub(crate) fn whitespace_free_parent(tag: &str) -> bool {
    matches!(
        tag,
        "ul" | "ol" | "menu" | "dl" | "table" | "thead" | "tbody" | "tfoot" | "tr" | "colgroup" | "select"
            | "optgroup" | "datalist" | "html" | "head" | "body" | "fieldset" | "details" | "dialog" | "figure"
    )
}

/// Es el padre uno en cuyo contenido no se deben tocar los espacios.
pub(crate) fn is_raw(tag: &str) -> bool {
    RAW.contains(&tag)
}

pub(crate) fn detect_eol(src: &str) -> &'static str {
    if src.contains("\r\n") { "\r\n" } else { "\n" }
}

/// Un nivel de sangría: tabulador si el archivo sangra con tabuladores; si no, el menor número
/// de espacios con que empieza una línea de etiquetas (2 si no hay ninguna sangrada).
pub(crate) fn detect_unit(src: &str) -> String {
    let mut min: Option<usize> = None;
    for line in src.lines() {
        let trimmed = line.trim_start_matches([' ', '\t']);
        if !trimmed.starts_with('<') || trimmed.len() == line.len() {
            continue;
        }
        let lead = &line[..line.len() - trimmed.len()];
        if lead.starts_with('\t') {
            return "\t".into();
        }
        min = Some(min.map_or(lead.len(), |m| m.min(lead.len())));
    }
    " ".repeat(min.filter(|n| (1..=8).contains(n)).unwrap_or(2))
}

fn line_start(src: &str, pos: usize) -> usize {
    src[..pos].rfind('\n').map_or(0, |i| i + 1)
}

/// La sangría (espacios y tabuladores iniciales) de la línea que contiene `pos`.
pub(crate) fn line_indent(src: &str, pos: usize) -> &str {
    let rest = &src[line_start(src, pos)..];
    let n = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    &rest[..n]
}

/// ¿Lo que hay en `pos` es lo primero de su línea (solo sangría antes)?
pub(crate) fn starts_line(src: &str, pos: usize) -> bool {
    src[line_start(src, pos)..pos].bytes().all(|b| b == b' ' || b == b'\t')
}

/// El rango a cortar para quitar `start..end` sin dejar una línea vacía: si lo cortado es lo
/// único de su(s) línea(s), se incluye la línea entera (sangría y salto); si comparte línea con
/// otro contenido, solo se corta el propio rango.
pub(crate) fn whole_lines(src: &str, start: usize, end: usize) -> (usize, usize) {
    if end > src.len() || !starts_line(src, start) {
        return (start, end);
    }
    let rest = &src[end..];
    let spaces = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    let after = &rest[spaces..];
    let newline = if after.starts_with("\r\n") {
        2
    } else if after.starts_with('\n') {
        1
    } else {
        return (start, end); // hay más contenido en esa línea
    };
    (line_start(src, start), end + spaces + newline)
}

/// El hueco de espacio en blanco que rodea `at`: dónde empieza y si contiene un salto de línea.
pub(crate) fn gap_around(src: &str, at: usize) -> (usize, bool) {
    let b = src.as_bytes();
    let ws = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0c);
    let mut start = at;
    while start > 0 && ws(b[start - 1]) {
        start -= 1;
    }
    let mut end = at;
    while end < b.len() && ws(b[end]) {
        end += 1;
    }
    (start, src[start..end].contains('\n'))
}

// ---------- Formato del fragmento ----------

enum Node {
    Text(String),
    Comment(String),
    Elem(El),
}

struct El {
    name: String,
    open: String,
    close: String,
    kids: Vec<Node>,
}

/// Fin (exclusivo) de la etiqueta que empieza en `rest`, sin contar `>` dentro de comillas.
fn tag_end(rest: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (i, b) in rest.bytes().enumerate() {
        match (quote, b) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(b),
            (None, b'>') => return Some(i + 1),
            _ => {}
        }
    }
    None
}

fn push(stack: &mut [El], root: &mut Vec<Node>, node: Node) {
    match stack.last_mut() {
        Some(top) => top.kids.push(node),
        None => root.push(node),
    }
}

/// Árbol de etiquetas de un fragmento ya serializado. `None` si algo no cuadra.
fn parse(html: &str) -> Option<Vec<Node>> {
    let mut stack: Vec<El> = Vec::new();
    let mut root: Vec<Node> = Vec::new();
    let mut i = 0;
    while i < html.len() {
        let rest = &html[i..];
        if rest.starts_with("<!--") {
            let end = rest.find("-->")? + 3;
            push(&mut stack, &mut root, Node::Comment(rest[..end].to_string()));
            i += end;
        } else if rest.starts_with("</") {
            let end = rest.find('>')? + 1;
            let name = rest[2..end - 1].trim().to_ascii_lowercase();
            let mut el = stack.pop()?;
            if el.name != name {
                return None;
            }
            el.close = rest[..end].to_string();
            push(&mut stack, &mut root, Node::Elem(el));
            i += end;
        } else if rest.starts_with('<') && rest[1..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            let end = tag_end(rest)?;
            let open = &rest[..end];
            let name: String = rest[1..]
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/')
                .collect::<String>()
                .to_ascii_lowercase();
            i += end;
            let el = El { name: name.clone(), open: open.to_string(), close: String::new(), kids: Vec::new() };
            if is_void(&name) || open.ends_with("/>") {
                push(&mut stack, &mut root, Node::Elem(el));
            } else if is_raw(&name) {
                let at = html[i..].to_ascii_lowercase().find(&format!("</{name}"))?;
                let gt = html[i + at..].find('>')? + 1;
                let mut el = el;
                if at > 0 {
                    el.kids.push(Node::Text(html[i..i + at].to_string()));
                }
                el.close = html[i + at..i + at + gt].to_string();
                i += at + gt;
                push(&mut stack, &mut root, Node::Elem(el));
            } else {
                stack.push(el);
            }
        } else {
            let first = rest.chars().next()?.len_utf8();
            let end = rest[first..].find('<').map_or(rest.len(), |p| p + first);
            push(&mut stack, &mut root, Node::Text(rest[..end].to_string()));
            i += end;
        }
    }
    stack.is_empty().then_some(root)
}

fn flat(n: &Node, out: &mut String) {
    match n {
        Node::Text(t) | Node::Comment(t) => out.push_str(t),
        Node::Elem(e) => {
            out.push_str(&e.open);
            e.kids.iter().for_each(|k| flat(k, out));
            out.push_str(&e.close);
        }
    }
}

fn blank(n: &Node) -> bool {
    matches!(n, Node::Text(t) if t.trim().is_empty())
}

/// ¿Se reparten los hijos de `e` en líneas? Sí si solo tiene elementos y partirlos no cambia lo
/// que se ve: en un contenedor sin espacio significativo, o con hijos que son bloques.
fn breaks(e: &El) -> bool {
    if is_raw(&e.name) {
        return false;
    }
    let kids: Vec<&Node> = e.kids.iter().filter(|n| !blank(n)).collect();
    if kids.is_empty() || kids.iter().any(|n| matches!(n, Node::Text(_))) {
        return false;
    }
    if whitespace_free_parent(&e.name) {
        return true;
    }
    if is_phrasing_tag(&e.name) || is_text_only_tag(&e.name) {
        return false;
    }
    kids.len() == 1 || kids.iter().all(|n| matches!(n, Node::Elem(c) if !is_phrasing_tag(&c.name)))
}

fn emit(n: &Node, base: &str, unit: &str, level: usize, eol: &str, out: &mut String) {
    match n {
        Node::Elem(e) if breaks(e) => {
            out.push_str(&e.open);
            for k in e.kids.iter().filter(|k| !blank(k)) {
                out.push_str(eol);
                out.push_str(base);
                out.push_str(&unit.repeat(level + 1));
                emit(k, base, unit, level + 1, eol, out);
            }
            out.push_str(eol);
            out.push_str(base);
            out.push_str(&unit.repeat(level));
            out.push_str(&e.close);
        }
        _ => flat(n, out),
    }
}

/// `html` (un fragmento serializado) con sus hijos repartidos en líneas. `base` es la sangría
/// de la línea donde empieza el fragmento; cada nivel interior suma `unit`. Si el fragmento no
/// se puede leer con seguridad, se devuelve tal cual.
pub(crate) fn format_fragment(html: &str, base: &str, unit: &str, eol: &str) -> String {
    let Some(nodes) = parse(html) else { return html.to_string() };
    let mut out = String::with_capacity(html.len() + 32);
    for n in &nodes {
        emit(n, base, unit, 0, eol, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_containers_are_split_one_child_per_line() {
        let out = format_fragment("<ul><li>Elemento</li><li>Dos</li></ul>", "  ", "  ", "\n");
        assert_eq!(out, "<ul>\n    <li>Elemento</li>\n    <li>Dos</li>\n  </ul>");
        let table = "<table border=\"1\"><tbody><tr><td>a</td><td>b</td></tr></tbody></table>";
        assert_eq!(
            format_fragment(table, "", "\t", "\r\n"),
            "<table border=\"1\">\r\n\t<tbody>\r\n\t\t<tr>\r\n\t\t\t<td>a</td>\r\n\t\t\t<td>b</td>\r\n\t\t</tr>\r\n\t</tbody>\r\n</table>"
        );
    }

    #[test]
    fn text_and_inline_content_is_never_split() {
        for keep in [
            "<p>Hola <b>mundo</b> y <i>más</i></p>",
            "<div>Texto</div>",
            "<div><b>a</b><i>b</i></div>",
            "<pre>  uno\n  dos</pre>",
            "<script>if (a < b) { x(\"</p>\"); }</script>",
            "<p>a &lt; b</p>",
            "<hr>",
        ] {
            assert_eq!(format_fragment(keep, "  ", "  ", "\n"), keep);
        }
    }

    #[test]
    fn an_unreadable_fragment_comes_back_untouched() {
        assert_eq!(format_fragment("<ul><li>sin cerrar</ul>", "", "  ", "\n"), "<ul><li>sin cerrar</ul>");
    }

    #[test]
    fn the_indent_unit_follows_the_file() {
        assert_eq!(detect_unit("<ul>\n    <li>a</li>\n</ul>"), "    ");
        assert_eq!(detect_unit("<ul>\n\t<li>a</li>\n</ul>"), "\t");
        assert_eq!(detect_unit("<ul><li>a</li></ul>"), "  ");
        assert_eq!(detect_unit("<div>\n  <p>a</p>\n    <p>b</p>\n</div>"), "  ");
        assert_eq!(detect_eol("a\r\nb"), "\r\n");
        assert_eq!(detect_eol("a\nb"), "\n");
    }

    #[test]
    fn line_helpers_find_indent_and_gaps() {
        let src = "<ul>\n  <li>A</li>\n  <li>B</li>\n</ul>";
        let b = src.rfind("<li>B").unwrap();
        assert_eq!(line_indent(src, b), "  ");
        assert!(starts_line(src, b) && !starts_line(src, b + 3));
        let (start, nl) = gap_around(src, b);
        assert_eq!(&src[..start], "<ul>\n  <li>A</li>");
        assert!(nl);
        assert!(!gap_around("<ul><li>a</li></ul>", 8).1);
    }
}
