//! Enlaces entre los archivos del sitio: encontrarlos (`href`/`src` en HTML, `url()` y
//! `@import` en CSS), saber a qué archivo apuntan y reescribirlos cuando ese archivo cambia
//! de nombre. Todo es texto puro: no toca el disco.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Html,
    Css,
}

/// Tipo de archivo que puede contener enlaces, según su extensión.
pub fn kind_of(rel: &str) -> Option<Kind> {
    let ext = rel.rsplit_once('.')?.1.to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => Some(Kind::Html),
        "css" => Some(Kind::Css),
        _ => None,
    }
}

/// Un enlace dentro de un texto: rango en bytes de su valor (sin comillas).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkRef {
    pub start: usize,
    pub end: usize,
    pub value: String,
}

/// Todos los enlaces del texto, en orden. En HTML son los atributos `href` y `src` de las
/// etiquetas (no `data-href`, ni texto suelto); en CSS, `url(...)` y `@import "…"`.
pub fn scan(text: &str, kind: Kind) -> Vec<LinkRef> {
    match kind {
        Kind::Html => scan_html(text),
        Kind::Css => scan_css(text),
    }
}

fn push_ref(out: &mut Vec<LinkRef>, text: &str, start: usize, end: usize) {
    if start < end {
        out.push(LinkRef { start, end, value: text[start..end].to_string() });
    }
}

fn scan_html(text: &str) -> Vec<LinkRef> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        if text[i..].starts_with("<!--") {
            i = text[i + 4..].find("-->").map_or(b.len(), |e| i + 4 + e + 3);
            continue;
        }
        i += 1;
        // Nombre de la etiqueta (o `!doctype`, `/p`…): no lleva atributos que nos importen.
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
            i += 1;
        }
        // Atributos hasta el `>` que cierra la etiqueta (fuera de comillas).
        while i < b.len() && b[i] != b'>' {
            while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
                i += 1;
            }
            let name_start = i;
            while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'>' | b'/') {
                i += 1;
            }
            let name = text[name_start..i].to_ascii_lowercase();
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= b.len() || b[i] != b'=' {
                continue;
            }
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            let (start, end) = match b.get(i) {
                Some(&q @ (b'"' | b'\'')) => {
                    let start = i + 1;
                    let end = text[start..].find(q as char).map_or(b.len(), |e| start + e);
                    i = (end + 1).min(b.len());
                    (start, end)
                }
                _ => {
                    let start = i;
                    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                        i += 1;
                    }
                    (start, i)
                }
            };
            if name == "href" || name == "src" {
                push_ref(&mut out, text, start, end);
            }
        }
    }
    out
}

fn scan_css(text: &str) -> Vec<LinkRef> {
    let b = text.as_bytes();
    let lower = text.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let rest = &lower[i..];
        if rest.starts_with("/*") {
            i = rest[2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
        } else if rest.starts_with("url(") {
            i += 4;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            let (start, end, next) = match b.get(i) {
                Some(&q @ (b'"' | b'\'')) => {
                    let start = i + 1;
                    let end = text[start..].find(q as char).map_or(b.len(), |e| start + e);
                    (start, end, end + 1)
                }
                _ => {
                    let start = i;
                    let end = text[start..].find(')').map_or(b.len(), |e| start + e);
                    (start, start + text[start..end].trim_end().len(), end)
                }
            };
            push_ref(&mut out, text, start, end);
            i = next.min(b.len());
        } else if rest.starts_with("@import") {
            i += "@import".len();
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if let Some(&q @ (b'"' | b'\'')) = b.get(i) {
                let start = i + 1;
                let end = text[start..].find(q as char).map_or(b.len(), |e| start + e);
                push_ref(&mut out, text, start, end);
                i = (end + 1).min(b.len());
            }
        } else {
            i += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    out
}

/// Ruta del sitio a la que apunta `value` visto desde el archivo `from_rel`: relativa a su
/// carpeta, o a la raíz si empieza por `/`. Sin `?consulta` ni `#ancla`, con `%XX`
/// decodificado. `None` si no es un archivo del sitio: URL con esquema, `//host`, solo un
/// ancla, vacío o una ruta que sube por encima de la raíz.
pub fn resolve(from_rel: &str, value: &str) -> Option<String> {
    let v = value.trim().split(['?', '#']).next().unwrap_or("");
    if v.is_empty() || v.starts_with("//") || v.contains('\\') {
        return None;
    }
    if v.split('/').next().is_some_and(|first| first.contains(':')) {
        return None; // https:, mailto:, data:, javascript:, C:…
    }
    let decoded = percent_decode(v)?;
    let mut parts: Vec<&str> = Vec::new();
    if !decoded.starts_with('/') {
        let mut dirs: Vec<&str> = from_rel.split('/').collect();
        dirs.pop();
        parts = dirs;
    }
    for seg in decoded.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// ¿`resolved` es `target` o, si `is_dir`, algo dentro de él? Se compara por segmentos
/// completos (`img` no es prefijo de `img2/…`) y sin distinguir mayúsculas (Windows).
pub fn points_into(resolved: &str, target: &str, is_dir: bool) -> bool {
    let (r, t) = (resolved.to_lowercase(), target.to_lowercase());
    r == t || (is_dir && r.strip_prefix(&t).is_some_and(|rest| rest.starts_with('/')))
}

/// Reescribe los enlaces de `text` (el archivo `from_old`, que pasará a llamarse/estar en
/// `from_new`) que apuntan a `old` —o dentro, si `is_dir`— para que apunten a `new`.
/// Devuelve el texto nuevo y cuántos enlaces cambió.
pub fn retarget(
    text: &str,
    kind: Kind,
    from_old: &str,
    from_new: &str,
    old: &str,
    new: &str,
    is_dir: bool,
) -> (String, usize) {
    let old_len = old.split('/').count();
    let mut out = text.to_string();
    let mut changed = 0;
    // De atrás hacia delante: reescribir un valor no mueve los rangos de los anteriores.
    for r in scan(text, kind).into_iter().rev() {
        let Some(resolved) = resolve(from_old, &r.value) else { continue };
        if !points_into(&resolved, old, is_dir) {
            continue;
        }
        let rest: Vec<&str> = resolved.split('/').skip(old_len).collect();
        let target = if rest.is_empty() { new.to_string() } else { format!("{new}/{}", rest.join("/")) };
        let suffix = &r.value[r.value.find(['?', '#']).unwrap_or(r.value.len())..];
        let lead = r.value.trim_start();
        let path = if lead.starts_with('/') {
            format!("/{}", encode_path(&target))
        } else {
            let rel = relative_from(from_new, &target);
            let dot = if lead.starts_with("./") && !rel.starts_with("../") { "./" } else { "" };
            format!("{dot}{}", encode_path(&rel))
        };
        let value = format!("{path}{suffix}");
        if value != r.value {
            out.replace_range(r.start..r.end, &value);
            changed += 1;
        }
    }
    (out, changed)
}

/// Ruta de `target` escrita desde la carpeta del archivo `from` (con `../` si hace falta).
fn relative_from(from: &str, target: &str) -> String {
    let mut dirs: Vec<&str> = from.split('/').collect();
    dirs.pop();
    let parts: Vec<&str> = target.split('/').collect();
    let common = dirs
        .iter()
        .zip(&parts)
        .take_while(|(d, p)| d.eq_ignore_ascii_case(p) && parts.len() > 1)
        .count()
        .min(parts.len() - 1);
    let mut segs: Vec<&str> = vec![".."; dirs.len() - common];
    segs.extend(&parts[common..]);
    segs.join("/")
}

/// Escribe `%XX` en los caracteres que romperían una URL (igual que `site::image_src`).
fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for ch in path.chars() {
        match ch {
            // `( ) '` también: un `)` cortaría `url(...)` sin comillas y un `'` cerraría `src='…'`.
            ' ' | '#' | '?' | '%' | '"' | '<' | '>' | '(' | ')' | '\'' => {
                out.push_str(&format!("%{:02X}", ch as u32));
            }
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix(text: &str, kind: Kind, old: &str, new: &str, is_dir: bool) -> (String, usize) {
        retarget(text, kind, "index.html", "index.html", old, new, is_dir)
    }

    #[test]
    fn retarget_renames_a_file_link_keeping_style_and_suffix() {
        let text = "<p>x</p><link href='css/a.css?v=2#x'><p>y</p>";
        let (out, n) = fix(text, Kind::Html, "css/a.css", "css/b.css", false);
        assert_eq!(out, "<p>x</p><link href='css/b.css?v=2#x'><p>y</p>");
        assert_eq!(n, 1);
        // Sin enlaces afectados, el texto queda idéntico.
        let (same, n) = fix(text, Kind::Html, "otro.css", "nuevo.css", false);
        assert_eq!((same.as_str(), n), (text, 0));
    }

    #[test]
    fn retarget_keeps_root_absolute_and_unquoted() {
        let (out, n) = fix("<link href=/css/a.css>", Kind::Html, "css/a.css", "css/b.css", false);
        assert_eq!((out.as_str(), n), ("<link href=/css/b.css>", 1));
    }

    #[test]
    fn retarget_encodes_spaces() {
        let (out, _) = fix("<img src=\"i/a.png\">", Kind::Html, "i/a.png", "i/mi foto.png", false);
        assert_eq!(out, "<img src=\"i/mi%20foto.png\">");
    }

    #[test]
    fn retarget_directory_rewrites_inner_paths_by_segment() {
        let text = "<img src=\"img/a/b.png\"><img src=\"img2/x.png\"><a href=\"img\">";
        let (out, n) = fix(text, Kind::Html, "img", "fotos", true);
        assert_eq!(out, "<img src=\"fotos/a/b.png\"><img src=\"img2/x.png\"><a href=\"fotos\">");
        assert_eq!(n, 2);
    }

    #[test]
    fn retarget_relative_from_a_subfolder() {
        let (out, n) = retarget(
            "<img src=\"../img/a.png\">", Kind::Html, "blog/p.html", "blog/p.html",
            "img/a.png", "img/b.png", false,
        );
        assert_eq!((out.as_str(), n), ("<img src=\"../img/b.png\">", 1));
    }

    #[test]
    fn retarget_moves_with_its_own_folder() {
        // La carpeta `a` pasa a llamarse `b`: el archivo `a/p.html` pasa a ser `b/p.html`.
        let text = "<img src=\"/a/x.png\"><img src=\"x.png\">";
        let (out, n) = retarget(text, Kind::Html, "a/p.html", "b/p.html", "a", "b", true);
        assert_eq!(out, "<img src=\"/b/x.png\"><img src=\"x.png\">");
        assert_eq!(n, 1, "el enlace relativo sigue siendo válido y no se toca");
    }

    #[test]
    fn retarget_css_url_and_import() {
        let css = "@import 'a.css'; p{background:url(\"img/a.png\")}";
        let (out, n) = fix(css, Kind::Css, "img", "fotos", true);
        assert_eq!(out, "@import 'a.css'; p{background:url(\"fotos/a.png\")}");
        assert_eq!(n, 1);
        let (out, n) = fix(css, Kind::Css, "a.css", "b.css", false);
        assert_eq!(out, "@import 'b.css'; p{background:url(\"img/a.png\")}");
        assert_eq!(n, 1);
    }

    #[test]
    fn retarget_escapes_characters_that_would_end_the_value() {
        // Un `)` rompería `url(...)` sin comillas y un `'` cerraría `src='…'`.
        let (out, _) = fix("p{background:url(img/a.png)}", Kind::Css, "img/a.png", "img/foto (1).png", false);
        assert_eq!(out, "p{background:url(img/foto%20%281%29.png)}");
        let (out, _) = fix("<img src='i/a.png'>", Kind::Html, "i/a.png", "i/l'image.png", false);
        assert_eq!(out, "<img src='i/l%27image.png'>");
        // Y lo escrito se vuelve a leer como el mismo archivo.
        assert_eq!(resolve("index.html", "img/foto%20%281%29.png").as_deref(), Some("img/foto (1).png"));
    }

    #[test]
    fn retarget_ignores_external() {
        let text = "<img src=\"https://x.com/img/a.png\"><a href=\"mailto:img\">";
        let (out, n) = fix(text, Kind::Html, "img", "fotos", true);
        assert_eq!((out.as_str(), n), (text, 0));
    }

    fn values(text: &str, kind: Kind) -> Vec<String> {
        scan(text, kind).into_iter().map(|r| r.value).collect()
    }

    #[test]
    fn kinds_follow_the_extension() {
        assert_eq!(kind_of("a.HTML"), Some(Kind::Html));
        assert_eq!(kind_of("x/b.htm"), Some(Kind::Html));
        assert_eq!(kind_of("c.css"), Some(Kind::Css));
        assert_eq!(kind_of("d.png"), None);
        assert_eq!(kind_of("css"), None);
    }

    #[test]
    fn scan_finds_href_and_src_in_all_quote_styles() {
        let text = r#"<a href="a.html"><img src='img/b.png'><link href=c.css>"#;
        let found = scan(text, Kind::Html);
        let vals: Vec<_> = found.iter().map(|r| r.value.as_str()).collect();
        assert_eq!(vals, ["a.html", "img/b.png", "c.css"]);
        for r in &found {
            assert_eq!(&text[r.start..r.end], r.value);
        }
        // `data-href` no es `href`; el texto fuera de etiquetas no cuenta.
        assert!(values(r#"<div data-href="x">href="y"</div>"#, Kind::Html).is_empty());
    }

    #[test]
    fn scan_css_finds_url_and_import() {
        let css = "@import \"base.css\"; a{background:url( 'img/x.png' )} b{background:url(y.png)}";
        assert_eq!(values(css, Kind::Css), ["base.css", "img/x.png", "y.png"]);
        let css = "@import url(\"otra.css\");";
        assert_eq!(values(css, Kind::Css), ["otra.css"]);
    }

    #[test]
    fn resolve_is_relative_to_the_containing_file() {
        assert_eq!(resolve("blog/post.html", "../css/a.css").as_deref(), Some("css/a.css"));
        assert_eq!(resolve("blog/post.html", "img/a.png").as_deref(), Some("blog/img/a.png"));
        assert_eq!(resolve("blog/post.html", "/css/a.css").as_deref(), Some("css/a.css"));
        assert_eq!(resolve("i.html", "./a.css").as_deref(), Some("a.css"));
    }

    #[test]
    fn resolve_strips_suffix_and_decodes() {
        assert_eq!(resolve("i.html", "mi%20foto.png?v=2#x").as_deref(), Some("mi foto.png"));
    }

    #[test]
    fn resolve_rejects_non_site_links() {
        for bad in [
            "https://x.com/a", "//cdn/a.css", "mailto:a@b", "data:text/css,p{}", "#top", "",
            "../../fuera.css", "javascript:void(0)",
        ] {
            assert_eq!(resolve("i.html", bad), None, "{bad}");
        }
    }

    #[test]
    fn points_into_matches_whole_segments() {
        assert!(points_into("img/a.png", "img", true));
        assert!(!points_into("img2/a.png", "img", true));
        assert!(!points_into("images/a.png", "img", true));
        assert!(points_into("IMG/a.png", "img", true));
        assert!(points_into("css/main.css", "css/main.css", false));
        assert!(points_into("css/MAIN.css", "css/main.css", false));
        assert!(!points_into("img/a.png", "img", false));
        assert!(points_into("img", "img", true));
    }
}
