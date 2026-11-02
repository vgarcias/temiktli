//! La carpeta del sitio: listar sus archivos, resolver rutas relativas sin salir de ella y
//! vincular hojas de estilo al `<head>`. Todo lo que llega desde los webviews como "ruta de
//! archivo" pasa por `resolve_in_root`.

use std::path::{Path, PathBuf};
use temiktli_core::{FileEntry, t};

const MAX_DEPTH: usize = 4;
const MAX_ENTRIES: usize = 800;
pub(crate) const SKIP_DIRS: &[&str] = &["node_modules", "target"];

/// Archivos y carpetas bajo `root`, en orden de árbol (carpetas antes que archivos).
/// Se omiten lo oculto, las copias `.bak`, carpetas pesadas y los enlaces simbólicos
/// (podrían apuntar fuera del sitio).
pub fn list_site(root: &Path) -> Vec<FileEntry> {
    let mut out = Vec::new();
    walk(root, "", 0, &mut out);
    out
}

fn walk(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<FileEntry>) {
    if depth > MAX_DEPTH || out.len() >= MAX_ENTRIES {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<(String, bool)> = read
        .filter_map(Result::ok)
        .filter_map(|e| {
            let ty = e.file_type().ok()?;
            if ty.is_symlink() {
                return None;
            }
            let name = e.file_name().into_string().ok()?;
            let skip = name.starts_with('.')
                || name.to_ascii_lowercase().ends_with(".bak")
                || (ty.is_dir() && SKIP_DIRS.contains(&name.as_str()));
            (!skip).then_some((name, ty.is_dir()))
        })
        .collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    for (name, is_dir) in entries {
        if out.len() >= MAX_ENTRIES {
            return;
        }
        let rel = format!("{prefix}{name}");
        out.push(FileEntry { path: rel.clone(), is_dir });
        if is_dir {
            walk(&dir.join(&name), &format!("{rel}/"), depth + 1, out);
        }
    }
}

/// Ruta absoluta del archivo `rel` (con `/`) si existe y, ya resuelta, está dentro de `root`.
/// Rechaza vacíos, `.`/`..`, rutas absolutas, unidades (`C:`) y barras invertidas.
pub fn resolve_in_root(root: &Path, rel: &str) -> Option<PathBuf> {
    if rel.is_empty() || rel.contains(['\\', ':']) || rel.starts_with('/') {
        return None;
    }
    if rel.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
        return None;
    }
    let root = root.canonicalize().ok()?;
    let full = root.join(rel).canonicalize().ok()?;
    (full.starts_with(&root) && full.is_file()).then_some(full)
}

/// Ruta relativa del sitio a la que apunta un `href` (ya decodificado), o `None` si no es un
/// archivo local de la carpeta: URLs (`http:`, `//cdn…`), rutas absolutas y todo lo que suba
/// con `..`. Se ignoran `?consulta` y `#ancla`, y un `./` inicial.
pub fn href_to_rel(href: &str) -> Option<String> {
    let mut h = href.trim();
    h = h.split(['?', '#']).next().unwrap_or("");
    while let Some(rest) = h.strip_prefix("./") {
        h = rest;
    }
    if h.is_empty() || h.starts_with('/') || h.contains(['\\', ':']) {
        return None;
    }
    h.split('/').all(|s| !s.is_empty() && s != "." && s != "..").then(|| h.to_string())
}

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg", "avif", "ico", "bmp"];

fn ext_of(rel: &str) -> String {
    rel.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

pub fn is_image(rel: &str) -> bool {
    IMAGE_EXTS.contains(&ext_of(rel).as_str())
}

/// Valor del atributo `src` para una imagen del sitio: la ruta relativa, con los caracteres
/// que romperían una URL escritos como `%XX` (el servidor del lienzo los decodifica).
pub fn image_src(rel: &str) -> String {
    let mut out = String::with_capacity(rel.len());
    for ch in rel.chars() {
        match ch {
            ' ' | '#' | '?' | '%' | '"' | '<' | '>' => out.push_str(&format!("%{:02X}", ch as u32)),
            _ => out.push(ch),
        }
    }
    out
}

/// Texto alternativo de partida: el nombre del archivo sin extensión y con espacios.
pub fn alt_from(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    stem.replace(['-', '_'], " ").trim().to_string()
}

const NEW_FILE_EXTS: &[&str] = &["css", "html", "htm"];
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
    "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

const MAX_NEW_DEPTH: usize = 8;

/// Cada componente debe ser un nombre válido en Windows (y nunca `.` ni `..`).
pub(crate) fn check_segments(parts: &[&str]) -> Result<(), String> {
    if parts.len() > MAX_NEW_DEPTH {
        return Err(t!("site.too_deep", max = MAX_NEW_DEPTH));
    }
    for part in parts {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        let bad_char = part.chars().any(|c| c.is_control() || "<>:\"|?*".contains(c));
        if part.is_empty() || *part == "." || *part == ".." || part.len() > 100 || bad_char
            || part.ends_with('.') || part.ends_with(' ') || RESERVED_NAMES.contains(&stem.as_str())
        {
            return Err(t!("site.invalid_name", name = part));
        }
    }
    Ok(())
}

/// ¿Lo escrito en el campo "nuevo" pide una carpeta? Sí si termina en `/` o si el último
/// nombre no tiene extensión (`img`, `assets/fuentes/`); si la tiene, es un archivo.
pub fn wants_folder(rel: &str) -> bool {
    let rel = rel.trim();
    rel.ends_with('/') || !rel.rsplit('/').next().unwrap_or("").contains('.')
}

/// Ruta absoluta de la carpeta nueva `rel`, creando las intermedias que falten. Cada
/// componente que ya existe debe ser una carpeta que, resuelta, siga dentro del sitio (un
/// enlace simbólico no puede sacar la creación de él). No se admite una que ya exista.
pub fn new_folder_target(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim().trim_end_matches('/');
    if rel.is_empty() || rel.starts_with('/') || rel.contains('\\') {
        return Err(t("site.relative_path_folder"));
    }
    let parts: Vec<&str> = rel.split('/').collect();
    check_segments(&parts)?;
    let root = root.canonicalize().map_err(|e| t!("site.root_unreadable", error = e))?;
    let mut base = root.clone();
    for (i, part) in parts.iter().enumerate() {
        let next = base.join(part);
        if !next.exists() {
            return Ok(base.join(parts[i..].join("/")));
        }
        base = next.canonicalize().map_err(|e| t!("site.unresolved", name = part, error = e))?;
        if !base.starts_with(&root) || !base.is_dir() {
            return Err(t!("site.not_a_site_folder", name = part));
        }
    }
    Err(t!("site.already_exists", path = rel))
}

/// Ruta absoluta donde crear el archivo nuevo `rel`, si es seguro: extensión permitida,
/// nombre válido en Windows, carpeta existente dentro del sitio y sin pisar nada.
pub fn new_file_target(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim();
    if !NEW_FILE_EXTS.contains(&ext_of(rel).as_str()) {
        return Err(t("site.only_css_html"));
    }
    if rel.starts_with('/') || rel.contains('\\') {
        return Err(t("site.relative_path_file"));
    }
    let parts: Vec<&str> = rel.split('/').collect();
    check_segments(&parts)?;
    let (name, dirs) = parts.split_last().expect("al menos un componente");
    let root = root.canonicalize().map_err(|e| t!("site.root_unreadable", error = e))?;
    let dir = root.join(dirs.join("/")).canonicalize().map_err(|_| t!("site.folder_missing", folder = dirs.join("/")))?;
    if !dir.starts_with(&root) || !dir.is_dir() {
        return Err(t("site.folder_outside"));
    }
    let target = dir.join(name);
    if target.exists() {
        return Err(t!("site.already_exists", path = rel));
    }
    Ok(target)
}

/// Contenido inicial de un archivo nuevo.
pub fn new_file_template(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    if ext_of(rel) == "css" {
        format!("/* {stem} */\n")
    } else {
        format!(
            "<!DOCTYPE html>\n<html lang=\"es\">\n<head>\n  <meta charset=\"utf-8\">\n  \
             <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n  \
             <title>{}</title>\n</head>\n<body>\n  \n</body>\n</html>\n",
            stem.replace('&', "&amp;").replace('<', "&lt;")
        )
    }
}

/// Clave para comparar rutas del sitio sin importar mayúsculas (Windows no las distingue).
pub fn rel_key(rel: &str) -> String {
    rel.to_lowercase()
}

fn escape_attr(v: &str) -> String {
    v.replace('&', "&amp;").replace('"', "&quot;")
}

/// ¿Alguna etiqueta `<link>` del documento apunta ya a `rel`?
pub fn is_linked(html: &str, rel: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    let want = rel.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find("<link") {
        let start = from + i;
        let end = lower[start..].find('>').map_or(lower.len(), |e| start + e);
        if lower[start..end].contains(&want) {
            return true;
        }
        from = end.max(start + 1);
    }
    false
}

/// Posición del `</head>` (no del `</header>`, que empieza igual). Es el primero del
/// documento: la cabecera siempre precede al cuerpo.
fn find_head_close(html: &str) -> Option<usize> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find("</head") {
        let at = from + i;
        let next = lower.as_bytes().get(at + "</head".len()).copied();
        if matches!(next, None | Some(b'>' | b' ' | b'\t' | b'\r' | b'\n' | b'/')) {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

/// Inserta `<link rel="stylesheet" href="…">` justo antes de `</head>`, en su propia línea
/// y con la sangría del cierre más dos espacios. `None` si el texto no tiene `</head>`.
pub fn insert_link(html: &str, href: &str) -> Option<String> {
    let at = find_head_close(html)?;
    let tag = format!("<link rel=\"stylesheet\" href=\"{}\">", escape_attr(href));
    let line_start = html[..at].rfind('\n').map_or(0, |i| i + 1);
    let lead = &html[line_start..at];
    let (pos, text) = if lead.trim().is_empty() {
        let eol = if html[..line_start].ends_with("\r\n") { "\r\n" } else { "\n" };
        (line_start, format!("{lead}  {tag}{eol}"))
    } else {
        (at, tag)
    };
    Some(format!("{}{}{}", &html[..pos], text, &html[pos..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn site(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("temiktli-site-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let root = base.join("site");
        fs::create_dir_all(root.join("css")).unwrap();
        fs::create_dir_all(root.join("img")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("node_modules")).unwrap();
        fs::write(root.join("index.html"), "<p>x</p>").unwrap();
        fs::write(root.join("Zeta.html"), "").unwrap();
        fs::write(root.join("index.html.bak"), "").unwrap();
        fs::write(root.join("css/main.css"), "p{}").unwrap();
        fs::write(root.join("img/a.png"), "").unwrap();
        fs::write(base.join("secreto.txt"), "no sirvas esto").unwrap();
        root
    }

    #[test]
    fn lists_in_tree_order_and_skips_noise() {
        let root = site("list");
        let paths: Vec<_> = list_site(&root).into_iter().map(|e| (e.path, e.is_dir)).collect();
        assert_eq!(
            paths,
            [
                ("css".to_string(), true),
                ("css/main.css".to_string(), false),
                ("img".to_string(), true),
                ("img/a.png".to_string(), false),
                ("index.html".to_string(), false),
                ("Zeta.html".to_string(), false),
            ]
        );
    }

    #[test]
    fn resolves_only_files_inside_the_site() {
        let root = site("resolve");
        assert!(resolve_in_root(&root, "css/main.css").is_some());
        assert!(resolve_in_root(&root, "index.html").is_some());
        for bad in [
            "", "css", "../secreto.txt", "css/../../secreto.txt", "./index.html", "/index.html",
            "C:/Windows/win.ini", "css\\main.css", "css//main.css", "nada.css", "css/",
        ] {
            assert!(resolve_in_root(&root, bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn link_is_inserted_on_its_own_line_before_head_closes() {
        let html = "<html>\n<head>\n  <title>T</title>\n</head>\n<body></body></html>";
        let out = insert_link(html, "css/main.css").unwrap();
        assert_eq!(
            out,
            "<html>\n<head>\n  <title>T</title>\n  <link rel=\"stylesheet\" href=\"css/main.css\">\n</head>\n<body></body></html>"
        );
        // Un <head> en una sola línea: la etiqueta va justo antes del cierre.
        let out = insert_link("<head><title>T</title></head>", "a&b.css").unwrap();
        assert_eq!(out, "<head><title>T</title><link rel=\"stylesheet\" href=\"a&amp;b.css\"></head>");
        // CRLF se respeta; sin </head> no se inventa nada.
        let out = insert_link("<head>\r\n</head>", "a.css").unwrap();
        assert_eq!(out, "<head>\r\n  <link rel=\"stylesheet\" href=\"a.css\">\r\n</head>");
        assert!(insert_link("<p>sin cabecera</p>", "a.css").is_none());
    }

    #[test]
    fn link_goes_in_head_not_in_a_header_element() {
        // Regresión: «</header>» empieza igual que «</head» y se confundía con él.
        let html = "<html>\n<head>\n<title>T</title>\n</head>\n<body>\n<header>\n  <h1>x</h1>\n</header>\n</body></html>";
        let out = insert_link(html, "a.css").unwrap();
        let link = out.find("<link").unwrap();
        assert!(link < out.find("</head>").unwrap(), "{out}");
        assert!(link < out.find("<body>").unwrap(), "{out}");
        assert_eq!(out.matches("<link").count(), 1);
        // Sin <head> pero con <header>: no hay dónde vincular.
        assert!(insert_link("<body><header></header></body>", "a.css").is_none());
        // «</head >» con espacio sí es el cierre.
        assert!(insert_link("<head></head >", "a.css").is_some());
    }

    #[test]
    fn href_becomes_a_site_path_only_when_it_is_a_local_file() {
        assert_eq!(href_to_rel("css/main.css").as_deref(), Some("css/main.css"));
        assert_eq!(href_to_rel("./css/main.css?v=3#x").as_deref(), Some("css/main.css"));
        assert_eq!(href_to_rel(" ././a.css ").as_deref(), Some("a.css"));
        for bad in [
            "", "https://cdn.example.com/a.css", "//cdn.example.com/a.css", "/a.css", "../a.css",
            "css/../../a.css", "C:/a.css", "css\\a.css", "css//a.css", "data:text/css,p{}", "?v=1",
        ] {
            assert_eq!(href_to_rel(bad), None, "{bad}");
        }
    }

    #[test]
    fn images_get_a_safe_src_and_a_readable_alt() {
        assert!(is_image("img/Foto.PNG") && is_image("a.svg") && !is_image("a.css") && !is_image("png"));
        assert_eq!(image_src("img/mi foto #1.png"), "img/mi%20foto%20%231.png");
        assert_eq!(image_src("a%b.png"), "a%25b.png");
        assert_eq!(alt_from("img/logo-principal_2024.png"), "logo principal 2024");
        assert_eq!(alt_from("sin-ext"), "sin ext");
    }

    #[test]
    fn new_files_are_created_only_where_it_is_safe() {
        let root = site("newfile");
        assert!(new_file_target(&root, "css/nuevo.css").is_ok());
        assert!(new_file_target(&root, "pagina.html").is_ok());
        for (bad, why) in [
            ("css/main.css", "ya existe"),
            ("INDEX.HTML", "ya existe (Windows no distingue mayúsculas)"),
            ("a.js", "extensión no permitida"),
            ("sin-ext", "sin extensión"),
            ("../fuera.css", "sube de carpeta"),
            ("css/../../fuera.css", "sube de carpeta"),
            ("/abs.css", "ruta absoluta"),
            ("C:/x.css", "unidad"),
            ("css\\x.css", "barra invertida"),
            ("noexiste/x.css", "carpeta inexistente"),
            ("css//x.css", "componente vacío"),
            ("con.css", "nombre reservado de Windows"),
            ("a<b.css", "carácter prohibido"),
        ] {
            assert!(new_file_target(&root, bad).is_err(), "{bad}: {why}");
        }
        assert!(new_file_target(&root, "css/").is_err());
    }

    #[test]
    fn the_new_field_tells_folders_from_files() {
        for folder in ["img", "img/", "assets/fuentes", "assets/fuentes/", " css "] {
            assert!(wants_folder(folder), "{folder}");
        }
        for file in ["a.css", "css/a.css", "pagina.html", "x.txt", ".oculto"] {
            assert!(!wants_folder(file), "{file}");
        }
    }

    #[test]
    fn new_folders_are_created_only_inside_the_site() {
        let root = site("newfolder");
        let canon = root.canonicalize().unwrap();
        assert_eq!(new_folder_target(&root, "fuentes").unwrap(), canon.join("fuentes"));
        assert_eq!(new_folder_target(&root, "fuentes/").unwrap(), canon.join("fuentes"));
        // Las intermedias que faltan se crean; las que existen se respetan.
        assert_eq!(new_folder_target(&root, "css/temas/oscuro").unwrap(), canon.join("css").join("temas/oscuro"));
        assert_eq!(new_folder_target(&root, "a/b/c").unwrap(), canon.join("a/b/c"));
        for (bad, why) in [
            ("css", "ya existe"),
            ("CSS/", "ya existe (Windows no distingue mayúsculas)"),
            ("index.html/sub", "un archivo no es carpeta"),
            ("../fuera", "sube de carpeta"),
            ("css/../../fuera", "sube de carpeta"),
            ("/abs", "ruta absoluta"),
            ("C:/x", "unidad"),
            ("a\\b", "barra invertida"),
            ("a//b", "componente vacío"),
            ("con", "nombre reservado"),
            ("a<b", "carácter prohibido"),
            ("", "vacío"),
            ("/", "vacío"),
            ("1/2/3/4/5/6/7/8/9", "demasiado profunda"),
        ] {
            assert!(new_folder_target(&root, bad).is_err(), "{bad}: {why}");
        }
    }

    #[test]
    fn new_file_templates_match_their_type() {
        assert_eq!(new_file_template("css/tema.css"), "/* tema */\n");
        let html = new_file_template("pagina.html");
        assert!(html.starts_with("<!DOCTYPE html>") && html.contains("<title>pagina</title>"));
        assert!(html.contains("<meta charset=\"utf-8\">"));
        assert!(new_file_template("a&b.html").contains("<title>a&amp;b</title>"));
    }

    #[test]
    fn detects_existing_links() {
        let html = "<head><LINK rel=stylesheet href='css/Main.css'><link rel=icon href=x.ico></head>";
        assert!(is_linked(html, "css/main.css"));
        assert!(!is_linked(html, "css/other.css"));
        assert!(!is_linked("<p>css/main.css</p>", "css/main.css"));
    }
}
