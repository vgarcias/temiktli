//! Reformateo de todo el HTML del documento: «Indentar código» (tabulaciones, un elemento de
//! bloque por línea, los elementos en línea dentro de la línea de su texto) y «Reducir código»
//! (nivel básico: sin comentarios ni espacios sobrantes). Trabajan sobre el texto, no sobre el
//! DOM, para no tocar atributos, entidades ni mayúsculas: solo cambian los espacios y saltos de
//! línea entre etiquetas y los comentarios.

use crate::dom::is_void;

// ---------- Etiquetas ----------

/// Elementos que ocupan su propia línea al indentar y delimitan los espacios al reducir.
/// Incluye los de la cabecera (`meta`, `link`, `title`, `script`, `style`…), donde los espacios
/// entre etiquetas tampoco se ven.
const BLOCK: &[&str] = &[
    "address", "article", "aside", "base", "blockquote", "body", "caption", "col", "colgroup", "dd",
    "details", "dialog", "div", "dl", "dt", "fieldset", "figcaption", "figure", "footer", "form",
    "h1", "h2", "h3", "h4", "h5", "h6", "head", "header", "hgroup", "hr", "html", "li", "link", "main",
    "menu", "meta", "nav", "noscript", "ol", "optgroup", "option", "p", "pre", "script", "section",
    "style", "summary", "table", "tbody", "td", "template", "tfoot", "th", "thead", "title", "tr", "ul",
];

/// Su contenido es texto tal cual (o casi): no se tocan sus espacios.
const VERBATIM: &[&str] = &["script", "style", "pre", "textarea"];

fn is_block(name: &str) -> bool {
    BLOCK.contains(&name)
}

/// ¿Abrir `new` cierra el elemento abierto `top` aunque no se haya escrito su cierre?
fn implied_close(top: &str, new: &str) -> bool {
    match top {
        "li" => new == "li",
        "dt" | "dd" => matches!(new, "dt" | "dd"),
        "td" | "th" => matches!(new, "td" | "th" | "tr"),
        "tr" => new == "tr",
        "option" => matches!(new, "option" | "optgroup"),
        "p" => matches!(
            new,
            "address" | "article" | "aside" | "blockquote" | "div" | "dl" | "fieldset" | "footer" | "form"
                | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "header" | "hr" | "main" | "nav" | "ol" | "p"
                | "pre" | "section" | "table" | "ul"
        ),
        _ => false,
    }
}

// ---------- Fichas (tokens) ----------

enum Tok<'a> {
    /// `<!DOCTYPE …>` y otras declaraciones.
    Doctype(&'a str),
    Comment(&'a str),
    Open { name: String, raw: &'a str },
    Close { name: String, raw: &'a str },
    Text(&'a str),
    /// `script`, `style`, `pre` o `textarea`: etiqueta de apertura, contenido y cierre.
    Verbatim { name: String, open: &'a str, body: &'a str, close: &'a str },
}

fn tag_name(s: &str) -> String {
    s.chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == ':')
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Longitud de la etiqueta que empieza en `s` (hasta su `>`, sin contar los de dentro de comillas).
fn tag_end(s: &str) -> usize {
    let mut quote: Option<u8> = None;
    for (i, &c) in s.as_bytes().iter().enumerate().skip(1) {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                b'"' | b'\'' => quote = Some(c),
                b'>' => return i + 1,
                _ => {}
            },
        }
    }
    s.len()
}

fn tokenize(html: &str) -> Vec<Tok<'_>> {
    let b = html.as_bytes();
    let mut toks = Vec::new();
    let (mut i, mut text_from) = (0, 0);
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &html[i..];
        let parsed: Option<(usize, Tok)> = if rest.starts_with("<!--") {
            let end = rest[4..].find("-->").map_or(rest.len(), |e| 4 + e + 3);
            Some((end, Tok::Comment(&rest[..end])))
        } else if rest.starts_with("<!") || rest.starts_with("<?") {
            let end = rest.find('>').map_or(rest.len(), |e| e + 1);
            Some((end, Tok::Doctype(&rest[..end])))
        } else if rest.starts_with("</") && rest[2..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            let end = rest.find('>').map_or(rest.len(), |e| e + 1);
            Some((end, Tok::Close { name: tag_name(&rest[2..]), raw: &rest[..end] }))
        } else if rest[1..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            let open_end = tag_end(rest);
            let name = tag_name(&rest[1..]);
            if VERBATIM.contains(&name.as_str()) {
                // Hasta el cierre del mismo nombre; si no existe, el resto es su contenido.
                let needle = format!("</{name}");
                let lower = rest.to_ascii_lowercase();
                let (body_end, close_end) = match lower[open_end..].find(&needle) {
                    Some(p) => {
                        let at = open_end + p;
                        (at, rest[at..].find('>').map_or(rest.len(), |e| at + e + 1))
                    }
                    None => (rest.len(), rest.len()),
                };
                let tok = Tok::Verbatim {
                    name,
                    open: &rest[..open_end],
                    body: &rest[open_end..body_end],
                    close: &rest[body_end..close_end],
                };
                Some((close_end, tok))
            } else {
                Some((open_end, Tok::Open { name, raw: &rest[..open_end] }))
            }
        } else {
            None // un `<` suelto es texto
        };
        match parsed {
            Some((len, tok)) => {
                if text_from < i {
                    toks.push(Tok::Text(&html[text_from..i]));
                }
                toks.push(tok);
                i += len;
                text_from = i;
            }
            None => i += 1,
        }
    }
    if text_from < b.len() {
        toks.push(Tok::Text(&html[text_from..]));
    }
    toks
}

// ---------- Árbol (para indentar) ----------

enum Node<'a> {
    Text(&'a str),
    Comment(&'a str),
    Doctype(&'a str),
    /// Una etiqueta de cierre sin apertura: se conserva tal cual.
    Stray(&'a str),
    Verbatim { name: String, open: &'a str, body: &'a str, close: &'a str },
    El(El<'a>),
}

struct El<'a> {
    name: String,
    open: &'a str,
    close: Option<&'a str>,
    void: bool,
    kids: Vec<Node<'a>>,
}

fn push_node<'a>(stack: &mut [El<'a>], root: &mut Vec<Node<'a>>, node: Node<'a>) {
    match stack.last_mut() {
        Some(top) => top.kids.push(node),
        None => root.push(node),
    }
}

fn pop_el<'a>(stack: &mut Vec<El<'a>>, root: &mut Vec<Node<'a>>, close: Option<&'a str>) {
    if let Some(mut el) = stack.pop() {
        el.close = close;
        push_node(stack, root, Node::El(el));
    }
}

fn build(toks: Vec<Tok<'_>>) -> Vec<Node<'_>> {
    let mut stack: Vec<El> = Vec::new();
    let mut root: Vec<Node> = Vec::new();
    for tok in toks {
        match tok {
            Tok::Text(t) => push_node(&mut stack, &mut root, Node::Text(t)),
            Tok::Comment(c) => push_node(&mut stack, &mut root, Node::Comment(c)),
            Tok::Doctype(d) => push_node(&mut stack, &mut root, Node::Doctype(d)),
            Tok::Verbatim { name, open, body, close } => {
                push_node(&mut stack, &mut root, Node::Verbatim { name, open, body, close });
            }
            Tok::Open { name, raw } => {
                while stack.last().is_some_and(|top| implied_close(&top.name, &name)) {
                    pop_el(&mut stack, &mut root, None);
                }
                // `<svg><path /></svg>`: una etiqueta que se cierra a sí misma no lleva hijos.
                let void = is_void(&name) || raw.ends_with("/>");
                let el = El { name, open: raw, close: None, void, kids: Vec::new() };
                if void {
                    push_node(&mut stack, &mut root, Node::El(el));
                } else {
                    stack.push(el);
                }
            }
            Tok::Close { name, raw } => match stack.iter().rposition(|e| e.name == name) {
                Some(pos) => {
                    while stack.len() > pos + 1 {
                        pop_el(&mut stack, &mut root, None); // sin cierre escrito
                    }
                    pop_el(&mut stack, &mut root, Some(raw));
                }
                None => push_node(&mut stack, &mut root, Node::Stray(raw)),
            },
        }
    }
    while !stack.is_empty() {
        pop_el(&mut stack, &mut root, None);
    }
    root
}

/// Ningún descendiente es de bloque: todo el elemento cabe en la línea de su texto.
fn flat(el: &El) -> bool {
    el.kids.iter().all(|k| match k {
        Node::Text(_) | Node::Comment(_) | Node::Stray(_) => true,
        Node::Doctype(_) => false,
        Node::Verbatim { name, .. } => name == "textarea",
        Node::El(c) => !is_block(&c.name) && flat(c),
    })
}

/// ¿Va en su propia línea (y no dentro de la línea de un texto)?
fn block_like(node: &Node) -> bool {
    match node {
        Node::Text(_) | Node::Comment(_) | Node::Stray(_) => false,
        Node::Doctype(_) => true,
        Node::Verbatim { name, .. } => name != "textarea",
        Node::El(e) => is_block(&e.name) || !flat(e),
    }
}

/// Añade `text` con cada tramo de espacios convertido en uno solo (también entre fichas).
fn push_collapsed(out: &mut String, text: &str) {
    for c in text.chars() {
        if c.is_ascii_whitespace() {
            if !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
}

fn inline_node(node: &Node, out: &mut String) {
    match node {
        Node::Text(t) => push_collapsed(out, t),
        Node::Comment(c) | Node::Stray(c) | Node::Doctype(c) => out.push_str(c),
        Node::Verbatim { open, body, close, .. } => {
            out.push_str(open);
            out.push_str(body);
            out.push_str(close);
        }
        Node::El(e) => {
            out.push_str(e.open);
            for k in &e.kids {
                inline_node(k, out);
            }
            if let Some(c) = e.close {
                out.push_str(c);
            }
        }
    }
}

fn indent_of(level: usize) -> String {
    "\t".repeat(level)
}

/// Líneas del contenido de un `<script>` o `<style>`, alineadas con tabulaciones en `level` y
/// conservando la sangría relativa que tuvieran entre sí.
fn reindent_block(body: &str, level: usize) -> Vec<String> {
    let lines: Vec<&str> = body.lines().map(str::trim_end).collect();
    let Some(first) = lines.iter().position(|l| !l.trim().is_empty()) else { return Vec::new() };
    let last = lines.iter().rposition(|l| !l.trim().is_empty()).unwrap_or(first);
    let lead = |l: &str| l.chars().take_while(|c| *c == ' ' || *c == '\t').count();
    let common = lines[first..=last].iter().filter(|l| !l.trim().is_empty()).map(|l| lead(l)).min().unwrap_or(0);
    lines[first..=last]
        .iter()
        .map(|l| if l.trim().is_empty() { String::new() } else { format!("{}{}", indent_of(level), &l[common..]) })
        .collect()
}

fn flush_run(run: &mut Vec<&Node>, level: usize, out: &mut Vec<String>) {
    let mut s = String::new();
    for n in run.iter() {
        inline_node(n, &mut s);
    }
    run.clear();
    let t = s.trim();
    if !t.is_empty() {
        out.push(format!("{}{t}", indent_of(level)));
    }
}

fn render(nodes: &[Node], level: usize, out: &mut Vec<String>) {
    let mut run: Vec<&Node> = Vec::new();
    for node in nodes {
        if block_like(node) {
            flush_run(&mut run, level, out);
            render_block(node, level, out);
        } else {
            run.push(node);
        }
    }
    flush_run(&mut run, level, out);
}

fn render_block(node: &Node, level: usize, out: &mut Vec<String>) {
    let ind = indent_of(level);
    match node {
        Node::Doctype(d) => out.push(format!("{ind}{}", d.trim())),
        Node::Verbatim { name, open, body, close } => {
            if name == "pre" {
                out.push(format!("{ind}{open}{body}{close}"));
                return;
            }
            let block = reindent_block(body, level + 1);
            if block.is_empty() {
                out.push(format!("{ind}{open}{close}"));
            } else {
                out.push(format!("{ind}{open}"));
                out.extend(block);
                if !close.is_empty() {
                    out.push(format!("{ind}{close}"));
                }
            }
        }
        Node::El(e) => {
            if e.void {
                out.push(format!("{ind}{}", e.open.trim()));
                return;
            }
            let has_content = e.kids.iter().any(|k| !matches!(k, Node::Text(t) if t.trim().is_empty()));
            let structural = matches!(e.name.as_str(), "html" | "head" | "body") && has_content;
            if flat(e) && !structural {
                let mut s = String::new();
                for k in &e.kids {
                    inline_node(k, &mut s);
                }
                out.push(format!("{ind}{}{}{}", e.open, s.trim(), e.close.unwrap_or("")));
            } else {
                out.push(format!("{ind}{}", e.open));
                render(&e.kids, level + 1, out);
                if let Some(c) = e.close {
                    out.push(format!("{ind}{c}"));
                }
            }
        }
        // El texto, los comentarios y las etiquetas sueltas van dentro de una línea.
        Node::Text(_) | Node::Comment(_) | Node::Stray(_) => {}
    }
}

/// ¿El archivo usa mayoritariamente `\r\n`?
fn uses_crlf(html: &str) -> bool {
    let crlf = html.matches("\r\n").count();
    crlf > 0 && crlf * 2 >= html.matches('\n').count()
}

/// Reparte el HTML en líneas con una tabulación por nivel.
pub fn indent_html(html: &str) -> String {
    let mut lines = Vec::new();
    render(&build(tokenize(html)), 0, &mut lines);
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    let out = out.replace("\r\n", "\n");
    if uses_crlf(html) { out.replace('\n', "\r\n") } else { out }
}

// ---------- Reducir ----------

/// Los comentarios condicionales de Internet Explorer cambian lo que se carga: se conservan.
fn is_conditional(comment: &str) -> bool {
    comment.starts_with("<!--[") || comment.contains("<![endif]")
}

/// ¿Los espacios pegados a esta ficha no se ven (límite de bloque)?
fn boundary(tok: &Tok) -> bool {
    match tok {
        Tok::Doctype(_) | Tok::Comment(_) => true,
        Tok::Verbatim { name, .. } => name != "textarea",
        Tok::Open { name, .. } | Tok::Close { name, .. } => is_block(name),
        Tok::Text(_) => false,
    }
}

/// La etiqueta con un solo espacio entre atributos y sin espacio antes de `>`, sin tocar lo que
/// va entre comillas.
fn compact_tag(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut quote: Option<char> = None;
    let (mut pending, mut prev) = (false, '<');
    for c in raw.chars() {
        if let Some(q) = quote {
            out.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        if c.is_ascii_whitespace() {
            pending = true;
            continue;
        }
        if pending && c != '>' {
            out.push(' ');
        }
        pending = false;
        out.push(c);
        if (c == '"' || c == '\'') && prev == '=' {
            quote = Some(c);
        }
        prev = c;
    }
    out
}

/// Quita comentarios y espacios y saltos de línea sobrantes, sin cambiar cómo se ve la página.
pub fn minify_html(html: &str) -> String {
    let toks = tokenize(html);
    let kept: Vec<&Tok> = toks.iter().filter(|t| !matches!(t, Tok::Comment(c) if !is_conditional(c))).collect();
    let mut out = String::with_capacity(html.len());
    for (i, tok) in kept.iter().enumerate() {
        match tok {
            Tok::Text(s) => {
                let before = i == 0 || boundary(kept[i - 1]);
                let after = i + 1 == kept.len() || boundary(kept[i + 1]);
                let mut text = String::new();
                push_collapsed(&mut text, s);
                let mut t = text.as_str();
                if before || out.ends_with(' ') {
                    t = t.trim_start();
                }
                if after {
                    t = t.trim_end();
                }
                out.push_str(t);
            }
            Tok::Open { raw, .. } | Tok::Close { raw, .. } => out.push_str(&compact_tag(raw)),
            Tok::Comment(c) | Tok::Doctype(c) => out.push_str(c),
            Tok::Verbatim { open, body, close, .. } => {
                out.push_str(&compact_tag(open));
                out.push_str(body);
                out.push_str(&compact_tag(close));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    /// Solo los caracteres que no son espacio, para comprobar que nada se perdió ni se movió.
    fn solid(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    const MESSY: &str = "<!DOCTYPE html>\n<html lang=\"es\">\n<head>\n  <meta charset=\"utf-8\">\n  <title> Mi página </title>\n  <style>\n    body { margin: 0 }\n      p { color: red }\n  </style>\n</head>\n<body>\n  <!-- cabecera -->\n  <header><h1>Hola   mundo</h1></header>\n  <p>Texto <a href=\"#\">enlace</a>\n   y <strong>más</strong>.</p>\n  <ul><li>uno<li>dos</ul>\n  <pre>  a\n    b</pre>\n  <textarea>\n x </textarea>\n  <script>\n    var a = 1;\n  </script>\n</body>\n</html>\n";

    // ---------- Indentar ----------

    #[test]
    fn indent_puts_blocks_on_their_own_lines_with_tabs() {
        let html = "<!DOCTYPE html><html><head><title>T</title></head><body><div><h1>Hola</h1><p>Texto <a href=\"#\">enlace</a> y <strong>más</strong></p></div></body></html>";
        assert_eq!(
            indent_html(html),
            "<!DOCTYPE html>\n<html>\n\t<head>\n\t\t<title>T</title>\n\t</head>\n\t<body>\n\t\t<div>\n\t\t\t<h1>Hola</h1>\n\t\t\t<p>Texto <a href=\"#\">enlace</a> y <strong>más</strong></p>\n\t\t</div>\n\t</body>\n</html>\n"
        );
    }

    #[test]
    fn indent_keeps_inline_elements_inside_the_text_line_and_collapses_spaces() {
        let out = indent_html("<body><p>\n   Hola   <b>a</b>\n mundo\n</p></body>");
        assert_eq!(out, "<body>\n\t<p>Hola <b>a</b> mundo</p>\n</body>\n");
    }

    #[test]
    fn indent_splits_text_and_blocks_when_they_are_mixed() {
        let out = indent_html("<body><div>Texto<p>x</p>más <b>y</b></div></body>");
        assert_eq!(out, "<body>\n\t<div>\n\t\tTexto\n\t\t<p>x</p>\n\t\tmás <b>y</b>\n\t</div>\n</body>\n");
    }

    #[test]
    fn indent_never_touches_pre_and_textarea() {
        let out = indent_html("<body><div><pre>  a\n    b</pre><p>x <textarea> q\n  r </textarea></p></div></body>");
        assert_eq!(
            out,
            "<body>\n\t<div>\n\t\t<pre>  a\n    b</pre>\n\t\t<p>x <textarea> q\n  r </textarea></p>\n\t</div>\n</body>\n"
        );
    }

    #[test]
    fn indent_reindents_style_and_script_blocks_keeping_their_text() {
        let out = indent_html(
            "<head><style>\n  body{margin:0}\n    p{color:red}\n</style><script>var a=1;</script><script src=\"a.js\"></script></head>",
        );
        assert_eq!(
            out,
            "<head>\n\t<style>\n\t\tbody{margin:0}\n\t\t  p{color:red}\n\t</style>\n\t<script>\n\t\tvar a=1;\n\t</script>\n\t<script src=\"a.js\"></script>\n</head>\n"
        );
    }

    #[test]
    fn indent_keeps_comments_and_handles_unclosed_and_stray_tags() {
        let out = indent_html("<body><!-- c --><ul><li>a<li>b</ul><div>x</span>y</div></body>");
        assert_eq!(
            out,
            "<body>\n\t<!-- c -->\n\t<ul>\n\t\t<li>a\n\t\t<li>b\n\t</ul>\n\t<div>x</span>y</div>\n</body>\n"
        );
    }

    #[test]
    fn indent_keeps_the_line_ending_style_of_the_file() {
        let crlf = indent_html("<body>\r\n<p>a</p>\r\n</body>\r\n");
        assert_eq!(crlf, "<body>\r\n\t<p>a</p>\r\n</body>\r\n");
        assert!(!crlf.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn indent_is_idempotent_and_loses_nothing() {
        let once = indent_html(MESSY);
        assert_eq!(indent_html(&once), once);
        assert_eq!(solid(&once), solid(MESSY));
        assert!(once.contains("\t<pre>  a\n    b</pre>"), "{once}");
    }

    #[test]
    fn indented_html_is_what_the_document_saves_back() {
        let out = indent_html(MESSY);
        let doc = Document::parse(&out);
        assert_eq!(doc.to_saved_html().html, out);
    }

    // ---------- Reducir ----------

    #[test]
    fn minify_drops_comments_and_spaces_between_blocks() {
        let html = "<!DOCTYPE html>\n<html>\n  <head>\n    <title> T </title>\n  </head>\n  <body>\n    <!-- c -->\n    <p>Hola   <b>a</b>\n mundo</p>\n  </body>\n</html>\n";
        assert_eq!(
            minify_html(html),
            "<!DOCTYPE html><html><head><title>T</title></head><body><p>Hola <b>a</b> mundo</p></body></html>"
        );
    }

    #[test]
    fn minify_keeps_the_space_between_inline_elements() {
        assert_eq!(minify_html("<p><b>a</b> <i>b</i></p>"), "<p><b>a</b> <i>b</i></p>");
        assert_eq!(minify_html("<p>a <b>b</b> c</p>"), "<p>a <b>b</b> c</p>");
    }

    #[test]
    fn minify_keeps_conditional_comments() {
        let html = "<head><!--[if IE]><p>x</p><![endif]--><!-- fuera --></head>";
        assert_eq!(minify_html(html), "<head><!--[if IE]><p>x</p><![endif]--></head>");
    }

    #[test]
    fn minify_never_touches_pre_textarea_script_and_style() {
        let html = "<body>\n <pre>  a\n  b</pre>\n <textarea>\n x </textarea>\n <script> var a = 1;\n </script>\n <style>\n p { color: red }\n </style>\n</body>";
        assert_eq!(
            minify_html(html),
            "<body><pre>  a\n  b</pre><textarea>\n x </textarea><script> var a = 1;\n </script><style>\n p { color: red }\n </style></body>"
        );
    }

    #[test]
    fn minify_tidies_spaces_inside_tags_but_not_inside_values() {
        assert_eq!(
            minify_html("<a   href=\"x  y\"\n   class='y' >t</a>"),
            "<a href=\"x  y\" class='y'>t</a>"
        );
        assert_eq!(minify_html("<br />"), "<br />");
    }

    #[test]
    fn minify_leaves_entities_and_stray_angle_brackets_alone() {
        assert_eq!(minify_html("<p>a &nbsp; b &lt; c < d</p>"), "<p>a &nbsp; b &lt; c < d</p>");
    }

    #[test]
    fn minify_is_idempotent_and_loses_no_text() {
        let once = minify_html(MESSY);
        assert_eq!(minify_html(&once), once);
        assert!(!once.contains("cabecera"), "{once}");
        assert!(once.len() < MESSY.len());
        assert_eq!(Document::parse(&once).to_saved_html().html, once);
    }

    #[test]
    fn minify_and_indent_roundtrip_to_the_same_page() {
        // Reducir lo indentado da lo mismo que reducir el original. (Sin `<style>` ni `<script>`:
        // indentar re-indenta su contenido y reducir no lo toca.)
        let page = "<!DOCTYPE html>\n<html>\n<head>\n  <title> Mi página </title>\n</head>\n<body>\n  <!-- c -->\n  <header><h1>Hola   mundo</h1></header>\n  <p>Texto <a href=\"#\">enlace</a>\n   y <strong>más</strong>.</p>\n  <ul><li>uno<li>dos</ul>\n  <pre>  a\n    b</pre>\n  <textarea>\n x </textarea>\n</body>\n</html>\n";
        assert_eq!(minify_html(&indent_html(page)), minify_html(page));
    }
}
