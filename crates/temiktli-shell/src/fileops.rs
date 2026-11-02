//! Operaciones de archivos del sitio que pueden romper enlaces: medir a quién afectan,
//! renombrar y enviar a la Papelera. La lógica vive aquí, sin ventanas, para poder probarla.

use std::path::{Path, PathBuf};

use crate::relink;
use crate::site;
use temiktli_core::t;

/// Un archivo del sitio que enlaza al elemento afectado, y cuántas veces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Referrer {
    pub rel: String,
    pub links: usize,
}

/// Lo que cambia (o se rompe) si se renombra o borra un elemento del sitio.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Impact {
    /// Archivos que enlazan al elemento (o a algo dentro, si es carpeta), por ruta.
    pub referrers: Vec<Referrer>,
    /// `.html/.htm/.css` que no se pudieron revisar (no UTF-8 o ilegibles).
    pub not_checked: usize,
    /// Archivos y subcarpetas que contiene el elemento (0 y 0 si es un archivo).
    pub files: usize,
    pub dirs: usize,
}

/// Todo lo que hay bajo `dir` (carpetas y archivos, con ruta del sitio), sin los límites de
/// profundidad y número de `site::list_site`. Se omiten lo oculto, las carpetas pesadas y
/// los enlaces simbólicos (podrían apuntar fuera del sitio).
fn walk_all(dir: &Path, prefix: &str, out: &mut Vec<(String, bool)>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<(String, bool)> = read
        .filter_map(Result::ok)
        .filter_map(|e| {
            let ty = e.file_type().ok()?;
            if ty.is_symlink() {
                return None;
            }
            let name = e.file_name().into_string().ok()?;
            let skip = name.starts_with('.') || (ty.is_dir() && site::SKIP_DIRS.contains(&name.as_str()));
            (!skip).then_some((name, ty.is_dir()))
        })
        .collect();
    entries.sort();
    for (name, is_dir) in entries {
        let rel = format!("{prefix}{name}");
        out.push((rel.clone(), is_dir));
        if is_dir {
            walk_all(&dir.join(&name), &format!("{rel}/"), out);
        }
    }
}

/// Qué archivos enlazan a `target_rel` (o a algo dentro, si `is_dir`) y qué contiene.
/// `ignore_inside` (borrar) descarta los archivos que están dentro del propio destino:
/// se van con él. No se revisa un `.html/.htm/.css` que no sea UTF-8 legible: se cuenta
/// en `not_checked` para avisar en vez de ignorarlo en silencio.
pub fn impact(root: &Path, target_rel: &str, is_dir: bool, ignore_inside: bool) -> Impact {
    let mut all = Vec::new();
    walk_all(root, "", &mut all);
    let mut im = Impact::default();
    for (rel, entry_is_dir) in &all {
        let inside = relink::points_into(rel, target_rel, true) && !rel.eq_ignore_ascii_case(target_rel);
        if is_dir && inside {
            if *entry_is_dir {
                im.dirs += 1;
            } else {
                im.files += 1;
            }
        }
        if *entry_is_dir {
            continue;
        }
        let Some(kind) = relink::kind_of(rel) else { continue };
        if ignore_inside && relink::points_into(rel, target_rel, is_dir) {
            continue;
        }
        let Some(text) = std::fs::read(root.join(rel)).ok().and_then(|b| String::from_utf8(b).ok()) else {
            im.not_checked += 1;
            continue;
        };
        let links = relink::scan(text.trim_start_matches('\u{feff}'), kind)
            .iter()
            .filter(|r| relink::resolve(rel, &r.value).is_some_and(|p| relink::points_into(&p, target_rel, is_dir)))
            .count();
        if links > 0 {
            im.referrers.push(Referrer { rel: rel.clone(), links });
        }
    }
    im.referrers.sort_by(|a, b| a.rel.cmp(&b.rel));
    im
}

/// Dónde van los elementos borrados. En la aplicación es la Papelera de Windows; en los
/// tests, una carpeta cualquiera (o un fallo), para no tocar la Papelera real.
pub trait Trash {
    fn send(&self, abs: &Path) -> Result<(), String>;
}

pub struct SystemTrash;

impl Trash for SystemTrash {
    fn send(&self, abs: &Path) -> Result<(), String> {
        trash::delete(abs).map_err(|e| t!("site.trash_failed", error = e))
    }
}

/// Ruta absoluta de `rel` (archivo o carpeta del sitio) si existe, está dentro de `root` y no
/// es un enlace simbólico (podría llevar fuera del sitio).
pub(crate) fn locate(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let bad = || t!("site.not_a_site_item", path = rel);
    if rel.is_empty()
        || rel.starts_with('/')
        || rel.contains(['\\', ':'])
        || rel.split('/').any(|s| s.is_empty() || s == "." || s == "..")
    {
        return Err(bad());
    }
    let root = root.canonicalize().map_err(|e| t!("site.root_unreadable", error = e))?;
    let abs = root.join(rel);
    let meta = std::fs::symlink_metadata(&abs).map_err(|_| bad())?;
    if meta.file_type().is_symlink() {
        return Err(bad());
    }
    let parent = abs.parent().and_then(|p| p.canonicalize().ok()).ok_or_else(bad)?;
    if !parent.starts_with(&root) {
        return Err(bad());
    }
    Ok(abs)
}

/// Lo que hay que saber de un renombrado antes de hacerlo.
#[derive(Debug, Clone)]
pub struct RenamePlan {
    pub from_rel: String,
    pub to_rel: String,
    pub from_abs: PathBuf,
    pub to_abs: PathBuf,
    pub is_dir: bool,
    /// El nombre nuevo es idéntico al actual.
    pub unchanged: bool,
    /// Un archivo cambia de extensión (incluido quitarla o añadirla).
    pub ext_changed: bool,
    /// Ya hay otro elemento con ese nombre en la carpeta.
    pub exists: bool,
    pub exists_is_dir: bool,
}

/// Prepara el renombrado de `rel` a `new_name` (solo el nombre, sin carpetas): valida el
/// nombre como los de Windows y mide lo que el usuario debe confirmar. No toca el disco.
pub fn plan_rename(root: &Path, rel: &str, new_name: &str) -> Result<RenamePlan, String> {
    let from_abs = locate(root, rel)?;
    if new_name.contains(['/', '\\']) {
        return Err(t("site.name_has_slash"));
    }
    site::check_segments(&[new_name])?;
    let current = rel.rsplit('/').next().unwrap_or(rel);
    let to_rel = format!("{}{new_name}", &rel[..rel.len() - current.len()]);
    let to_abs = from_abs.with_file_name(new_name);
    let is_dir = from_abs.is_dir();
    let unchanged = new_name == current;
    let case_only = !unchanged && new_name.to_lowercase() == current.to_lowercase();
    // Cambiar solo mayúsculas apunta al mismo elemento: no es «ya existe».
    let exists = !unchanged && !case_only && std::fs::symlink_metadata(&to_abs).is_ok();
    let ext = |n: &str| n.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
    Ok(RenamePlan {
        from_rel: rel.to_string(),
        to_rel,
        exists_is_dir: exists && to_abs.is_dir(),
        ext_changed: !is_dir && ext(current) != ext(new_name),
        from_abs,
        to_abs,
        is_dir,
        unchanged,
        exists,
    })
}

/// Hace el renombrado en disco. Si el nombre ya existe solo sigue con `replace`, y entonces
/// el existente va a la Papelera (si eso falla no se renombra nada).
pub fn apply_rename(plan: &RenamePlan, replace: bool, trash: &dyn Trash) -> Result<(), String> {
    if plan.unchanged {
        return Ok(());
    }
    if plan.exists {
        if !replace {
            return Err(t!("site.already_exists", path = plan.to_rel));
        }
        trash.send(&plan.to_abs)?;
    }
    std::fs::rename(&plan.from_abs, &plan.to_abs).map_err(|e| t!("site.rename_failed", path = plan.from_rel, error = e))
}

#[derive(Debug, Default)]
pub struct RelinkReport {
    pub updated: Vec<String>,
    pub failed: Vec<(String, String)>,
}

/// Tras renombrar en disco: reescribe en disco los enlaces de los demás archivos. `skip`
/// son rutas (tras el renombrado) de archivos abiertos con cambios en memoria.
pub fn relink_site(root: &Path, plan: &RenamePlan, skip: &[String]) -> RelinkReport {
    let mut all = Vec::new();
    walk_all(root, "", &mut all);
    let mut report = RelinkReport::default();
    for (rel, entry_is_dir) in all {
        let Some(kind) = relink::kind_of(&rel).filter(|_| !entry_is_dir) else { continue };
        if skip.iter().any(|s| site::rel_key(s) == site::rel_key(&rel)) {
            continue;
        }
        // Los no UTF-8 ya se avisaron en `Impact::not_checked`.
        let abs = root.join(&rel);
        let Some(text) = std::fs::read(&abs).ok().and_then(|b| String::from_utf8(b).ok()) else { continue };
        let (body, bom) = match text.strip_prefix('\u{feff}') {
            Some(b) => (b, true),
            None => (text.as_str(), false),
        };
        let from_old = location_before(plan, &rel);
        let (new_body, n) = relink::retarget(body, kind, &from_old, &rel, &plan.from_rel, &plan.to_rel, plan.is_dir);
        if n == 0 {
            continue;
        }
        let out = if bom { format!("\u{feff}{new_body}") } else { new_body };
        match std::fs::write(&abs, out) {
            Ok(()) => report.updated.push(rel),
            Err(e) => report.failed.push((rel, e.to_string())),
        }
    }
    report
}

/// Dónde estaba `rel` antes del renombrado: si el elemento renombrado es él o una carpeta que
/// lo contiene, su ruta cambió; el resto no se movió.
fn location_before(plan: &RenamePlan, rel: &str) -> String {
    if relink::points_into(rel, &plan.to_rel, plan.is_dir) {
        format!("{}{}", plan.from_rel, rel.get(plan.to_rel.len()..).unwrap_or(""))
    } else {
        rel.to_string()
    }
}

/// Envía `rel` (archivo o carpeta del sitio) a la Papelera. Si no se puede, no se borra por
/// ninguna otra vía: el error vuelve tal cual.
pub fn delete_to_trash(root: &Path, rel: &str, trash: &dyn Trash) -> Result<(), String> {
    trash.send(&locate(root, rel)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::fs;

    /// Papelera de mentira: mueve a una carpeta temporal (o falla) y recuerda qué recibió.
    struct FakeTrash {
        dir: PathBuf,
        fail: bool,
        got: RefCell<Vec<PathBuf>>,
    }

    impl FakeTrash {
        fn new(root: &Path, fail: bool) -> Self {
            let dir = root.parent().unwrap().join("papelera");
            fs::create_dir_all(&dir).unwrap();
            FakeTrash { dir, fail, got: RefCell::new(Vec::new()) }
        }
    }

    impl Trash for FakeTrash {
        fn send(&self, abs: &Path) -> Result<(), String> {
            if self.fail {
                return Err("la Papelera no está disponible".into());
            }
            self.got.borrow_mut().push(abs.to_path_buf());
            let name = format!("{}-{}", self.got.borrow().len(), abs.file_name().unwrap().to_string_lossy());
            fs::rename(abs, self.dir.join(name)).map_err(|e| e.to_string())
        }
    }

    fn site(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("temiktli-fileops-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let root = base.join("site");
        for d in ["css", "img", "blog", ".git"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        fs::write(root.join("css/main.css"), "p{}").unwrap();
        fs::write(root.join("img/a.png"), "").unwrap();
        root
    }

    #[test]
    fn impact_lists_files_that_link_to_a_file() {
        let root = site("file");
        fs::write(root.join("index.html"), r#"<link href="css/main.css">"#).unwrap();
        fs::write(root.join("blog/p.html"), r#"<link href="../css/main.css"><a href="x.html">"#).unwrap();
        fs::write(root.join("otra.html"), "<p>nada</p>").unwrap();
        let im = impact(&root, "css/main.css", false, false);
        assert_eq!(
            im.referrers,
            [
                Referrer { rel: "blog/p.html".into(), links: 1 },
                Referrer { rel: "index.html".into(), links: 1 },
            ]
        );
        assert_eq!((im.not_checked, im.files, im.dirs), (0, 0, 0));
    }

    #[test]
    fn impact_of_a_folder_counts_inner_links_and_contents() {
        let root = site("dir");
        fs::create_dir_all(root.join("img/sub")).unwrap();
        fs::write(root.join("img/sub/b.png"), "").unwrap();
        fs::write(
            root.join("index.html"),
            r#"<img src="img/a.png"><img src="img/sub/b.png"><img src="img2/c.png">"#,
        )
        .unwrap();
        let im = impact(&root, "img", true, true);
        assert_eq!(im.referrers, [Referrer { rel: "index.html".into(), links: 2 }]);
        assert_eq!((im.files, im.dirs), (2, 1));
    }

    #[test]
    fn delete_ignores_links_from_inside_the_target() {
        let root = site("inside");
        fs::write(root.join("img/leeme.html"), r#"<img src="a.png"><img src="/img/a.png">"#).unwrap();
        assert!(impact(&root, "img", true, true).referrers.is_empty());
        let im = impact(&root, "img", true, false);
        // `a.png` (relativo) y `/img/a.png` (absoluto) apuntan los dos a `img/a.png`.
        assert_eq!(im.referrers, [Referrer { rel: "img/leeme.html".into(), links: 2 }]);
    }

    #[test]
    fn impact_reports_unreadable_files() {
        let root = site("utf8");
        fs::write(root.join("mal.html"), [0xff, 0xfe, 0x41]).unwrap();
        fs::write(root.join("bien.html"), r#"<link href="css/main.css">"#).unwrap();
        let im = impact(&root, "css/main.css", false, false);
        assert_eq!(im.not_checked, 1);
        assert_eq!(im.referrers.len(), 1);
    }

    #[test]
    fn impact_skips_hidden_folders_and_reads_a_bom() {
        let root = site("hidden");
        fs::write(root.join(".git/x.html"), r#"<link href="css/main.css">"#).unwrap();
        fs::write(root.join("bom.html"), "\u{feff}<link href=\"css/main.css\">").unwrap();
        let im = impact(&root, "css/main.css", false, false);
        assert_eq!(im.referrers, [Referrer { rel: "bom.html".into(), links: 1 }]);
    }

    #[test]
    fn impact_goes_deeper_than_the_panel_listing() {
        let root = site("deep");
        let deep = root.join("a/b/c/d/e/f");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("p.html"), r#"<link href="../../../../../../css/main.css">"#).unwrap();
        let im = impact(&root, "css/main.css", false, false);
        assert_eq!(im.referrers, [Referrer { rel: "a/b/c/d/e/f/p.html".into(), links: 1 }]);
    }

    #[test]
    fn plan_rename_flags() {
        let root = site("plan");
        fs::write(root.join("css/a.css"), "").unwrap();
        let plan = plan_rename(&root, "css/main.css", "tema.css").unwrap();
        assert_eq!(plan.to_rel, "css/tema.css");
        assert!(!plan.ext_changed && !plan.exists && !plan.unchanged && !plan.is_dir);
        assert!(plan_rename(&root, "css/main.css", "tema.txt").unwrap().ext_changed);
        assert!(plan_rename(&root, "css/main.css", "tema").unwrap().ext_changed);
        assert!(plan_rename(&root, "css/main.css", "main.css").unwrap().unchanged);
        let case = plan_rename(&root, "css/main.css", "Main.css").unwrap();
        // Solo cambian las mayúsculas: es el mismo elemento, no «ya existe».
        assert!(!case.exists && !case.unchanged);
        let taken = plan_rename(&root, "css/main.css", "a.css").unwrap();
        assert!(taken.exists && !taken.exists_is_dir);
        // Una carpeta no tiene «extensión» que avisar, aunque el nombre lleve un punto.
        let dir = plan_rename(&root, "img", "fotos.v2").unwrap();
        assert!(dir.is_dir && !dir.ext_changed);
        fs::create_dir_all(root.join("css/sub")).unwrap();
        assert!(plan_rename(&root, "css/main.css", "sub").unwrap().exists_is_dir);
    }

    #[test]
    fn plan_rename_rejects_bad_names_and_paths() {
        let root = site("planbad");
        for bad in ["", "a/b.css", "a\\b.css", "a<b.css", "con.css", "..", ".", "x.", "x "] {
            assert!(plan_rename(&root, "css/main.css", bad).is_err(), "{bad:?}");
        }
        for bad_rel in ["", "nada.css", "../fuera.css", "css/../../x", "/css/main.css", "css//main.css", "C:/x"] {
            assert!(plan_rename(&root, bad_rel, "otro.css").is_err(), "{bad_rel:?}");
        }
    }

    #[test]
    fn apply_rename_moves_a_file_and_a_folder() {
        let root = site("apply");
        let t = FakeTrash::new(&root, false);
        apply_rename(&plan_rename(&root, "css/main.css", "tema.css").unwrap(), false, &t).unwrap();
        assert!(root.join("css/tema.css").is_file() && !root.join("css/main.css").exists());
        apply_rename(&plan_rename(&root, "img", "fotos").unwrap(), false, &t).unwrap();
        assert!(root.join("fotos/a.png").is_file() && !root.join("img").exists());
        assert!(t.got.borrow().is_empty());
    }

    #[test]
    fn apply_rename_case_only_works() {
        let root = site("case");
        let t = FakeTrash::new(&root, false);
        let plan = plan_rename(&root, "css/main.css", "MAIN.css").unwrap();
        apply_rename(&plan, false, &t).unwrap();
        let names: Vec<_> = fs::read_dir(root.join("css")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["MAIN.css"]);
        assert!(t.got.borrow().is_empty(), "no hay nada que reemplazar");
    }

    #[test]
    fn apply_rename_replace_sends_existing_to_trash() {
        let root = site("replace");
        fs::write(root.join("css/otro.css"), "viejo").unwrap();
        let plan = plan_rename(&root, "css/main.css", "otro.css").unwrap();
        // Sin permiso para reemplazar: error y nada cambia.
        let t = FakeTrash::new(&root, false);
        assert!(apply_rename(&plan, false, &t).is_err());
        assert_eq!(fs::read_to_string(root.join("css/otro.css")).unwrap(), "viejo");
        assert!(root.join("css/main.css").exists());
        // Con la Papelera rota: error y el origen sigue donde estaba.
        let broken = FakeTrash::new(&root, true);
        assert!(apply_rename(&plan, true, &broken).is_err());
        assert!(root.join("css/main.css").exists());
        assert_eq!(fs::read_to_string(root.join("css/otro.css")).unwrap(), "viejo");
        // Con permiso: el existente va a la Papelera y el renombrado ocupa su sitio.
        apply_rename(&plan, true, &t).unwrap();
        assert_eq!(fs::read_to_string(root.join("css/otro.css")).unwrap(), "p{}");
        assert!(!root.join("css/main.css").exists());
        assert_eq!(t.got.borrow().len(), 1);
    }

    #[test]
    fn apply_rename_replace_works_across_types() {
        let root = site("types");
        let t = FakeTrash::new(&root, false);
        // Un archivo sobre una carpeta (las dos en `css`).
        fs::create_dir_all(root.join("css/carpeta")).unwrap();
        let plan = plan_rename(&root, "css/main.css", "carpeta").unwrap();
        assert!(plan.exists && plan.exists_is_dir);
        apply_rename(&plan, true, &t).unwrap();
        assert!(root.join("css/carpeta").is_file() && !root.join("css/main.css").exists());
        // Una carpeta sobre un archivo (las dos en la raíz).
        fs::write(root.join("pagina.html"), "").unwrap();
        let plan = plan_rename(&root, "img", "pagina.html").unwrap();
        assert!(plan.exists && !plan.exists_is_dir);
        apply_rename(&plan, true, &t).unwrap();
        assert!(root.join("pagina.html").is_dir() && root.join("pagina.html/a.png").is_file());
    }

    #[test]
    fn relink_site_updates_other_files_on_disk() {
        let root = site("relink");
        fs::write(root.join("index.html"), "<p>\r\n</p><link href=\"css/main.css\">\r\n").unwrap();
        fs::write(root.join("blog/p.html"), "<link href='../css/main.css?v=1'>").unwrap();
        fs::write(root.join("abierto.html"), "<link href=\"css/main.css\">").unwrap();
        fs::write(root.join("bom.html"), "\u{feff}<link href=\"css/main.css\">").unwrap();
        fs::write(root.join("estilo.css"), "@import 'css/main.css';").unwrap();
        let t = FakeTrash::new(&root, false);
        let plan = plan_rename(&root, "css/main.css", "tema.css").unwrap();
        apply_rename(&plan, false, &t).unwrap();
        let report = relink_site(&root, &plan, &["Abierto.html".to_string()]);
        assert_eq!(report.updated, ["blog/p.html", "bom.html", "estilo.css", "index.html"]);
        assert!(report.failed.is_empty());
        let read = |p: &str| fs::read_to_string(root.join(p)).unwrap();
        assert_eq!(read("index.html"), "<p>\r\n</p><link href=\"css/tema.css\">\r\n");
        assert_eq!(read("blog/p.html"), "<link href='../css/tema.css?v=1'>");
        assert_eq!(read("bom.html"), "\u{feff}<link href=\"css/tema.css\">");
        assert_eq!(read("estilo.css"), "@import 'css/tema.css';");
        assert_eq!(read("abierto.html"), "<link href=\"css/main.css\">", "lo abierto lo trata la aplicación");
    }

    #[test]
    fn relink_site_folder_rename_fixes_inner_absolute_links() {
        let root = site("relinkdir");
        fs::write(root.join("img/p.html"), "<img src=\"/img/a.png\"><img src=\"a.png\">").unwrap();
        fs::write(root.join("index.html"), "<img src=\"img/a.png\">").unwrap();
        let t = FakeTrash::new(&root, false);
        let plan = plan_rename(&root, "img", "fotos").unwrap();
        apply_rename(&plan, false, &t).unwrap();
        let report = relink_site(&root, &plan, &[]);
        assert_eq!(report.updated, ["fotos/p.html", "index.html"]);
        let read = |p: &str| fs::read_to_string(root.join(p)).unwrap();
        assert_eq!(read("fotos/p.html"), "<img src=\"/fotos/a.png\"><img src=\"a.png\">");
        assert_eq!(read("index.html"), "<img src=\"fotos/a.png\">");
    }

    #[test]
    fn relink_site_reports_failures_and_continues() {
        let root = site("relinkfail");
        fs::write(root.join("index.html"), "<link href=\"css/main.css\">").unwrap();
        fs::write(root.join("blog/p.html"), "<link href=\"../css/main.css\">").unwrap();
        let mut perms = fs::metadata(root.join("index.html")).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(root.join("index.html"), perms).unwrap();
        let t = FakeTrash::new(&root, false);
        let plan = plan_rename(&root, "css/main.css", "tema.css").unwrap();
        apply_rename(&plan, false, &t).unwrap();
        let report = relink_site(&root, &plan, &[]);
        assert_eq!(report.updated, ["blog/p.html"]);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, "index.html");
        let mut perms = fs::metadata(root.join("index.html")).unwrap().permissions();
        perms.set_readonly(false);
        fs::set_permissions(root.join("index.html"), perms).unwrap();
    }

    #[test]
    fn delete_to_trash_calls_trash_and_never_removes() {
        let root = site("delete");
        let t = FakeTrash::new(&root, false);
        delete_to_trash(&root, "img", &t).unwrap();
        assert!(!root.join("img").exists());
        assert_eq!(t.got.borrow().len(), 1);
        // Si la Papelera falla, el archivo sigue ahí: nunca se borra por otra vía.
        let broken = FakeTrash::new(&root, true);
        assert!(delete_to_trash(&root, "css/main.css", &broken).is_err());
        assert!(root.join("css/main.css").is_file());
        // Fuera del sitio o inexistente: error, y la Papelera ni se toca.
        for bad in ["../x", "nada.css", "", "/css/main.css"] {
            assert!(delete_to_trash(&root, bad, &t).is_err(), "{bad:?}");
        }
        assert_eq!(t.got.borrow().len(), 1);
    }
}
