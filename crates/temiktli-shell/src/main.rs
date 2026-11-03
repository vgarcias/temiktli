//! Temiktli, Fase 1: ventana tao con dos webviews hijos. El lienzo muestra la página
//! (proyección del DOM de Rust) y el panel ofrece Propiedades e Insertar. Toda
//! edición pasa por un `Command` y vuelve al lienzo como `Patch`.

// Sin consola en Windows al abrir la aplicación; en desarrollo (debug) se conserva para ver los logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod fileops;
mod relink;
mod site;
mod snippets;

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use tao::dpi::LogicalSize;
use tao::event::{ElementState, Event, MouseButton, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{CursorIcon, Window, WindowBuilder};
use temiktli_core::{
    Command, DeleteNode, Document, FileEntry, FromWebview, History, InsertElement, MoveNode, NodeId, OpenCss,
    PanelState, Patch, ReplaceDocument, RuleView, SaveMode, SetAttr, SetText, Sheet, SiteEdit,
    SourceIndex, Category, ContrastItem, FixAction, Issue, ReviewCounts, Severity, i18n,
    parse_rules, rule_views_for, set_decl, t, tn, tnr,
};
use wry::dpi::{PhysicalPosition, PhysicalSize, Position, Size};
use wry::http::{Request, Response, header::CONTENT_TYPE};
use wry::{Rect, WebView, WebViewBuilder};

/// Documento nuevo: con el que arranca la aplicación y el que queda al cerrar el archivo. Su
/// `lang` es el idioma de las páginas que crea el editor (bloque `content` del idioma activo).
fn blank_html() -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"{}\">\n<head>\n<meta charset=\"utf-8\">\n<title></title>\n</head>\n<body>\n</body>\n</html>\n",
        t("content.document_lang")
    )
}
const INJECT_JS: &str = include_str!("../assets/editor-inject.js");
const I18N_JS: &str = include_str!("../assets/i18n.js");
const PANEL_HTML_SRC: &str = include_str!("../assets/panel.html");
const TOOLBAR_HTML: &str = include_str!("../assets/toolbar.html");
const POPUP_HTML_SRC: &str = include_str!("../assets/popup.html");
const CODE_HTML: &str = include_str!("../assets/code.html");
const HIGHLIGHT_JS: &str = include_str!("../assets/highlight.js");
const COMPLETE_JS: &str = include_str!("../assets/complete.js");
const FIND_JS: &str = include_str!("../assets/find.js");

/// La vista de código con coloreador y autocompletado incluidos (un solo documento, sin
/// cargas externas).
fn code_page() -> String {
    with_i18n(CODE_HTML)
        .replacen("/*HIGHLIGHT_JS*/", HIGHLIGHT_JS, 1)
        .replacen("/*COMPLETE_JS*/", COMPLETE_JS, 1)
        .replacen("/*FIND_JS*/", FIND_JS, 1)
}

/// El panel (un solo documento, sin cargas externas).
fn panel_page() -> String {
    with_i18n(PANEL_HTML_SRC)
}

/// La barra de menús.
fn toolbar_page() -> String {
    with_i18n(TOOLBAR_HTML)
}

/// Los menús desplegables, con el motor de búsqueda y el catálogo de etiquetas incluidos
/// (los nombres de las categorías ya en el idioma activo).
fn popup_page() -> String {
    let categories = temiktli_core::tag_menu_json().to_string();
    with_i18n(POPUP_HTML_SRC)
        .replacen("/*FIND_JS*/", FIND_JS, 1)
        // El marcador lleva su valor vacío (`[]`): se sustituyen juntos.
        .replacen("/*TAGS_JSON*/[]", &categories, 1)
}

/// Los textos del idioma activo (con el inglés debajo) incrustados en `i18n.js`.
fn i18n_js() -> String {
    I18N_JS.replacen("/*I18N_JSON*/{}", &i18n::flat_json(), 1)
}

/// Una página de webview con su `i18n.js` puesto en el marcador `/*I18N_JS*/`.
fn with_i18n(page: &str) -> String {
    page.replacen("/*I18N_JS*/", &i18n_js(), 1)
}

/// El script del lienzo con sus (pocos) textos en el idioma activo.
fn inject_js() -> String {
    INJECT_JS.replacen("/*I18N_JSON*/{}", &i18n::flat_json(), 1)
}


const PANEL_WIDTH_LOGICAL: f64 = 340.0;
const CANVAS_PATH: &str = "/__canvas.html";

/// WebView2 expone los protocolos propios como `http://<nombre>.localhost`.
#[cfg(windows)]
const ORIGIN: &str = "http://tk.localhost";
#[cfg(not(windows))]
const ORIGIN: &str = "tk://localhost";


// ---------- Servidor del protocolo tk:// ----------

#[derive(Default)]
struct Shared {
    canvas_html: String,
    /// Carpeta del documento: único lugar desde el que se sirven recursos.
    site_root: Option<PathBuf>,
    /// Hojas de estilo abiertas en la vista de código: lo que el lienzo debe ver aunque
    /// aún no estén guardadas. Clave: `site::rel_key` de la ruta relativa.
    overrides: HashMap<String, String>,
}

type SharedRef = Arc<Mutex<Shared>>;

fn canvas_url(version: u32) -> String {
    format!("{ORIGIN}{CANVAS_PATH}?v={version}")
}

fn serve(shared: &SharedRef, req: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let path = percent_decode(req.uri().path());
    let shared = shared.lock().expect("estado compartido");

    if path == CANVAS_PATH {
        // El lienzo nunca ejecuta los scripts de la página del usuario: CSP los bloquea
        // (incluidos los manejadores en línea como onclick). El script del editor se
        // inyecta desde el host y no depende de la página.
        return Response::builder()
            .status(200)
            .header(CONTENT_TYPE, "text/html; charset=utf-8")
            .header("Content-Security-Policy", "script-src 'none'")
            .header("Cache-Control", "no-store")
            .body(Cow::Owned(shared.canvas_html.clone().into_bytes()))
            .expect("respuesta");
    }

    if let Some(css) = shared.overrides.get(&site::rel_key(path.trim_start_matches('/'))) {
        return Response::builder()
            .status(200)
            .header(CONTENT_TYPE, "text/css; charset=utf-8")
            .header("Cache-Control", "no-store")
            .body(Cow::Owned(css.clone().into_bytes()))
            .expect("respuesta");
    }

    match read_site_file(shared.site_root.as_deref(), &path) {
        Some((bytes, mime)) => Response::builder()
            .status(200)
            .header(CONTENT_TYPE, mime)
            .header("Cache-Control", "no-store")
            .body(Cow::Owned(bytes))
            .expect("respuesta"),
        None => Response::builder().status(404).body(Cow::Borrowed(&b"not found"[..])).expect("respuesta"),
    }
}

/// Lee un archivo del sitio. Rechaza cualquier ruta que, ya resuelta, salga de la raíz.
fn read_site_file(root: Option<&Path>, url_path: &str) -> Option<(Vec<u8>, &'static str)> {
    let full = site::resolve_in_root(root?, url_path.trim_start_matches('/'))?;
    let bytes = std::fs::read(&full).ok()?;
    Some((bytes, mime_for(&full)))
}

fn mime_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Línea y columna (desde 0, en unidades UTF-16 como las de JavaScript) de un byte de `text`.
fn line_col_utf16(text: &str, byte: usize) -> (usize, usize) {
    let before = &text[..byte];
    let line = before.matches('\n').count();
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, before[start..].encode_utf16().count())
}

/// Byte de `text` que corresponde a la posición `offset` en unidades UTF-16 (acotada al final).
fn utf16_to_byte(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (i, c) in text.char_indices() {
        if units >= offset {
            return i;
        }
        units += c.len_utf16();
    }
    text.len()
}

/// Lo que ve el menú de cada snippet: su nombre y si es de serie (esos no se borran). El HTML
/// no viaja: el menú solo devuelve la posición del que se elige.
fn snippet_menu(all: &[snippets::Snippet], builtin_count: usize) -> Vec<serde_json::Value> {
    all.iter()
        .enumerate()
        .map(|(i, s)| serde_json::json!({ "name": s.name, "builtin": i < builtin_count }))
        .collect()
}

/// Mensaje de la barra de estado tras guardar: avisa si el archivo se reformateó entero.
fn save_status(target: &Path, backup: Option<&Path>, mode: &SaveMode) -> String {
    let mut msg = t!("status.saved", path = target.display());
    match mode {
        SaveMode::Preserved => msg.push_str(&t("status.saved_preserved")),
        SaveMode::Normalized(why) => msg.push_str(&t!("status.saved_normalized", why = why)),
    }
    if let Some(name) = backup.and_then(Path::file_name) {
        msg.push_str(&t!("status.saved_backup", name = name.to_string_lossy()));
    }
    msg
}

/// Guarda una copia byte a byte del archivo antes de que Temiktli lo reescriba normalizado.
/// Devuelve la ruta de la copia si la creó. Nunca pisa un `.bak` existente: así la copia
/// sigue siendo el original aunque se guarde varias veces. Un error debe impedir guardar.
fn backup_original(target: &Path) -> Result<Option<PathBuf>, String> {
    if !target.is_file() {
        return Ok(None); // archivo nuevo: no hay nada que proteger
    }
    let mut name = target.file_name().ok_or_else(|| t("status.save_path_without_name"))?.to_os_string();
    name.push(".bak");
    let bak = target.with_file_name(name);
    if bak.exists() {
        return Ok(None);
    }
    std::fs::copy(target, &bak).map_err(|e| e.to_string())?;
    Ok(Some(bak))
}

/// Ruta de `file` relativa a `root` (con `/`), si existe y está dentro. Ambas se canonicalizan:
/// la del panel viene de una raíz resuelta (`\\?\D:\…`) y la del documento no.
fn rel_within(root: &Path, file: &Path) -> Option<String> {
    let file = file.canonicalize().ok()?;
    let rel = file.strip_prefix(root.canonicalize().ok()?).ok()?;
    Some(rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"))
}

/// Pregunta «Aceptar / Cancelar» con aviso; `true` si el usuario acepta.
fn confirm(title: &str, text: String) -> bool {
    MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(title)
        .set_description(text)
        .set_buttons(MessageButtons::OkCancel)
        .show()
        == MessageDialogResult::Ok
}

/// «2,0 KB → 1,0 KB (-50 %)»: cuánto ocupaba el HTML y cuánto ocupa ahora.
fn size_summary(before: usize, after: usize) -> String {
    let size = |n: usize| {
        if n < 1024 {
            t!("units.bytes", n = n)
        } else {
            let kb = format!("{:.1}", n as f64 / 1024.0).replace('.', &i18n::decimal_separator());
            t!("units.kilobytes", n = kb)
        }
    };
    let change = if before == after || before == 0 {
        t("units.size_unchanged")
    } else {
        let pct = ((after as f64 - before as f64) / before as f64 * 100.0).round() as i64;
        t!("units.percent", sign = if pct > 0 { "+" } else { "" }, pct = pct)
    };
    t!("units.size_change", before = size(before), after = size(after), change = change)
}

/// Texto del diálogo de borrado: qué va a la Papelera, qué enlaces se rompen y qué se pierde.
fn delete_summary(rel: &str, is_dir: bool, im: &fileops::Impact, dirty_css: usize) -> String {
    let mut s = if is_dir {
        t!("dialog.delete_folder", path = rel, files = tn("units.file", im.files), folders = tn("units.folder", im.dirs))
    } else {
        t!("dialog.delete_file", path = rel)
    };
    s.push_str(&t("dialog.delete_recoverable"));
    if !im.referrers.is_empty() {
        let links: usize = im.referrers.iter().map(|r| r.links).sum();
        s.push_str(&t!("dialog.delete_breaks", links = tn("units.link", links), files = tn("units.file", im.referrers.len())));
        s.push_str(&referrer_lines(&im.referrers));
    }
    if im.not_checked > 0 {
        s.push_str(&t!("dialog.delete_unchecked", files = tn("units.file", im.not_checked)));
    }
    if dirty_css > 0 {
        s.push_str(&tnr("dialog.delete_dirty", dirty_css, &[("sheets", &tn("units.sheet", dirty_css))]));
    }
    s
}

/// Nueva ruta de un archivo abierto (`open`) cuando se renombra `old` a `new`: él mismo o una
/// carpeta que lo contiene. `None` si el renombrado no le afecta.
fn rename_open_path(open: &str, old: &str, new: &str, is_dir: bool) -> Option<String> {
    relink::points_into(open, old, is_dir).then(|| format!("{new}{}", open.get(old.len()..).unwrap_or("")))
}

/// Texto del diálogo «¿Reemplazar?»: avisa de las hojas abiertas con cambios sin guardar que
/// se cerrarían con lo reemplazado.
fn replace_text(plan: &fileops::RenamePlan, dirty_css: usize) -> String {
    let what = t(if plan.exists_is_dir { "dialog.replace_what_folder" } else { "dialog.replace_what_file" });
    let mut s = t!("dialog.replace_text", what = what, path = plan.to_rel);
    if dirty_css > 0 {
        s.push_str(&tnr("dialog.replace_dirty", dirty_css, &[("sheets", &tn("units.sheet", dirty_css))]));
    }
    s
}

/// Aviso cuando nadie enlaza (que sepamos) pero hay archivos que no se pudieron revisar.
fn unchecked_text(plan: &fileops::RenamePlan, not_checked: usize) -> String {
    t!("dialog.rename_only_unchecked", from = plan.from_rel, to = plan.to_rel, unchecked = tn("units.unchecked", not_checked))
}

/// Actualiza la ruta de las hojas en los pasos de deshacer/rehacer tras un renombrado.
fn remap_steps(steps: &mut [Step], from: &str, to: &str, is_dir: bool) {
    for step in steps {
        if let Step::Css(edit) = step {
            if let Some(new_rel) = rename_open_path(&edit.rel, from, to, is_dir) {
                edit.rel = new_rel;
            }
        }
    }
}

/// Texto del diálogo de renombrado cuando otros archivos enlazan al elemento. Los botones
/// son los estándar de Windows (Sí / No / Cancelar), así que el texto dice qué hace cada uno.
fn rename_summary(plan: &fileops::RenamePlan, im: &fileops::Impact) -> String {
    let what = t(if plan.is_dir { "dialog.rename_what_folder" } else { "dialog.rename_what_file" });
    let mut s = t!("dialog.rename_question", what = what, from = plan.from_rel, to = plan.to_rel);
    let links: usize = im.referrers.iter().map(|r| r.links).sum();
    s.push_str(&t!("dialog.rename_links", links = tn("units.link", links), files = tn("units.file", im.referrers.len())));
    s.push_str(&referrer_lines(&im.referrers));
    if im.not_checked > 0 {
        s.push_str(&t!("dialog.rename_unchecked", unchecked = tn("units.unchecked", im.not_checked)));
    }
    s.push_str(&t("dialog.rename_buttons"));
    s
}

/// Líneas « • archivo (n)» de los primeros archivos que enlazan, y cuántos más hay.
fn referrer_lines(referrers: &[fileops::Referrer]) -> String {
    const MAX_LISTED: usize = 8;
    let mut s = String::new();
    for r in referrers.iter().take(MAX_LISTED) {
        s.push_str(&t!("dialog.referrer_line", file = r.rel, links = r.links));
    }
    if referrers.len() > MAX_LISTED {
        s.push_str(&t!("dialog.referrer_more", n = referrers.len() - MAX_LISTED));
    }
    s
}

// ---------- Aplicación ----------

struct App {
    doc: Document,
    history: History,
    selection: Option<NodeId>,
    path: Option<PathBuf>,
    dirty: bool,
    status: String,
    shared: SharedRef,
    load_version: u32,
    /// Ancho lógico del lienzo para probar diseño responsivo (`None` = todo el espacio).
    viewport: Option<u32>,
    /// "system" | "light" | "dark": el `prefers-color-scheme` que ve la página.
    theme: String,
    /// Idioma elegido en el menú ("system" o un código), para marcarlo; el activo ya está
    /// cargado en `i18n` y solo cambia al reiniciar.
    language: String,
    /// Dónde se guarda `config.json`; `None` si no hay carpeta de datos.
    config_path: Option<PathBuf>,
    /// Reglas de todos los `<style>` tal como se enviaron al lienzo para comprobar coincidencias.
    probed_rules: Vec<RuleView>,
    /// De ellas, las que afectan al elemento seleccionado (respuesta del lienzo).
    matched_rules: Vec<RuleView>,
    /// Vista de código HTML bajo el lienzo; siempre disponible, visible por defecto.
    code: WebView,
    code_open: bool,
    arrangement: Arrangement,
    /// Últimas zonas aplicadas a los webviews (y cuáles son visibles), para no repetir trabajo.
    last_layout: Option<(Zones, (bool, bool), Option<Px>)>,
    /// Menú desplegable abierto (`insert`, `view`, `search` o `review`) y dónde empieza (lógico).
    open_menu: Option<String>,
    menu_x: f64,
    /// Tamaño (lógico) que el menú abierto dijo necesitar.
    menu_dims: Option<(f64, f64)>,
    /// Último menú que se cerró y cuándo: pulsar su botón justo después significa «cerrar».
    menu_closed: Option<(String, Instant)>,
    /// Elemento copiado: su texto original y cuántos elementos contiene.
    clipboard: Option<(String, usize)>,
    /// Snippets propios del usuario y el archivo donde se guardan.
    user_snippets: Vec<snippets::Snippet>,
    snippets_path: Option<PathBuf>,
    toolbar: WebView,
    popup: WebView,
    /// Cuánto ocupa el código en cada disposición; el usuario lo cambia arrastrando el divisor.
    fractions: Fractions,
    /// El ratón está sobre el divisor / se está arrastrando.
    over_divider: bool,
    dragging: bool,
    /// Sube con cada cambio del documento; invalida `span_index`.
    doc_version: u32,
    /// Última revisión y la versión del documento sobre la que se hizo (`None` = sin
    /// revisión vigente). Los ids que contiene solo valen mientras el documento no cambie.
    review: Option<(u32, Vec<Issue>)>,
    /// Revisar el documento automáticamente después de guardar.
    auto_review: bool,
    /// Dónde está cada elemento en `code_text` (versión del documento, largo del texto, mapa).
    /// Construirlo recorre todo el HTML, así que se reutiliza mientras nada cambie.
    span_index: Option<(u32, usize, Option<SourceIndex>)>,
    /// El documento cambió desde el último envío a la vista de código.
    code_dirty: bool,
    /// El próximo envío debe pisar el texto aunque el usuario tenga el foco allí.
    code_force: bool,
    /// Último texto enviado (o recibido) de la vista de código.
    code_text: String,
    /// Hojas de estilo externas abiertas como pestañas de la vista de código.
    css_files: Vec<CssBuf>,
    /// Pestaña activa (ruta relativa del `.css`); `None` = el documento HTML.
    active_file: Option<String>,
    /// Contenido de la carpeta del sitio y si el panel necesita recibirlo de nuevo.
    site_files: Vec<FileEntry>,
    files_dirty: bool,
    /// Selección (línea, columna, largo) que la vista de código aplica al mostrar su próximo texto.
    pending_goto: Option<(usize, usize, usize)>,
    /// Orden en que se hicieron los cambios deshacibles (documento y hojas externas).
    undo_log: Vec<Step>,
    redo_log: Vec<Step>,
    canvas: WebView,
    panel: WebView,
}

/// Un `.css` abierto: lo editado en memoria frente a lo último guardado en disco.
struct CssBuf {
    rel: String,
    abs: PathBuf,
    text: String,
    saved: String,
}

/// Un paso del deshacer unificado: un comando del documento (que guarda `History`) o un
/// cambio de declaración en un `.css` externo hecho desde el panel Estilos.
enum Step {
    Doc,
    Css(CssEdit),
}

struct CssEdit {
    rel: String,
    before: String,
    after: String,
}

impl CssBuf {
    fn dirty(&self) -> bool {
        self.text != self.saved
    }
}

/// Anchos que ofrece el selector del panel. Cualquier otro valor se rechaza.
const VIEWPORT_PRESETS: &[u32] = &[375, 768, 1024];

fn eval(view: &WebView, js: &str) {
    if let Err(e) = view.evaluate_script(js) {
        eprintln!("evaluate_script falló: {e}");
    }
}

impl App {
    fn file_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| t("app.new_file_name"))
    }

    /// ¿Hay algo sin guardar, en el documento o en alguna hoja de estilo abierta?
    fn any_dirty(&self) -> bool {
        self.dirty || self.css_files.iter().any(CssBuf::dirty)
    }

    fn title(&self) -> String {
        format!("{}{}", if self.any_dirty() { "● " } else { "" }, t!("app.window_title", file = self.file_name()))
    }

    fn state(&mut self) -> PanelState {
        // La lista de archivos solo viaja cuando cambió.
        let files = std::mem::take(&mut self.files_dirty).then(|| self.site_files.clone());
        PanelState {
            selection: self.selection.and_then(|id| self.doc.selection_info(id)),
            can_undo: !self.undo_log.is_empty(),
            can_redo: !self.redo_log.is_empty(),
            can_paste: self.clipboard.is_some(),
            review: self.review_counts(),
            dirty: self.any_dirty(),
            file_name: self.file_name(),
            status: self.status.clone(),
            viewport: self.viewport,
            theme: self.theme.clone(),
            language: self.language.clone(),
            rules: self.matched_rules.clone(),
            code_open: self.code_open,
            layout: self.arrangement.name().to_string(),
            files,
            has_folder: self.path.is_some(),
            css_open: self
                .css_files
                .iter()
                .map(|c| OpenCss { path: c.rel.clone(), dirty: c.dirty() })
                .collect(),
            active_file: self.active_file.clone(),
        }
    }

    fn push_state(&mut self) {
        let json = serde_json::to_string(&self.state()).expect("json");
        eval(&self.panel, &format!("window.__panel && window.__panel.update({json})"));
        eval(&self.toolbar, &format!("window.__toolbar && window.__toolbar.update({json})"));
        eval(&self.popup, &format!("window.__popup && window.__popup.update({json})"));
    }

    // ---------- Carpeta del sitio y hojas de estilo ----------

    fn site_root(&self) -> Option<PathBuf> {
        self.path.as_deref().and_then(Path::parent).map(Path::to_path_buf)
    }

    fn refresh_files(&mut self) {
        self.site_files = self.site_root().map(|r| site::list_site(&r)).unwrap_or_default();
        self.files_dirty = true;
    }

    /// Deja en `Shared` el contenido de las hojas abiertas, para que el lienzo vea lo editado.
    fn sync_overrides(&self) {
        let mut s = self.shared.lock().expect("estado compartido");
        s.overrides = self.css_files.iter().map(|c| (site::rel_key(&c.rel), c.text.clone())).collect();
    }

    /// El lienzo vuelve a pedir sus hojas de estilo (sin recargar la página ni perder el scroll).
    fn refresh_canvas_css(&self) {
        eval(&self.canvas, "window.__tk && window.__tk.refreshCss()");
    }

    /// Posición de la hoja abierta `rel` (sin distinguir mayúsculas: Windows tampoco).
    fn css_index(&self, rel: &str) -> Option<usize> {
        let key = site::rel_key(rel);
        self.css_files.iter().position(|c| site::rel_key(&c.rel) == key)
    }

    /// Carga una hoja de estilo del sitio como pestaña (sin activarla) si aún no lo está.
    /// Devuelve su posición, o `None` explicando en la barra de estado por qué no.
    fn load_css(&mut self, rel: &str) -> Option<usize> {
        if let Some(i) = self.css_index(rel) {
            return Some(i);
        }
        let Some(root) = self.site_root() else {
            self.status = t("status.css_no_folder");
            return None;
        };
        if !rel.to_ascii_lowercase().ends_with(".css") {
            self.status = t("status.css_only");
            return None;
        }
        let Some(abs) = site::resolve_in_root(&root, rel) else {
            self.status = t!("status.cannot_open", path = rel);
            return None;
        };
        match std::fs::read(&abs).map(String::from_utf8) {
            Ok(Ok(text)) => {
                let text = text.trim_start_matches('\u{feff}').to_string();
                self.css_files.push(CssBuf { rel: rel.to_string(), abs, saved: text.clone(), text });
                self.sync_overrides();
                Some(self.css_files.len() - 1)
            }
            Ok(Err(_)) => {
                self.status = t!("status.not_utf8", path = rel);
                None
            }
            Err(e) => {
                self.status = t!("status.cannot_read", path = rel, error = e);
                None
            }
        }
    }

    fn open_css(&mut self, rel: &str) {
        let Some(i) = self.load_css(rel) else { return };
        // La vista de código tiene que estar a la vista para editar.
        self.show_code();
        let rel = self.css_files[i].rel.clone();
        self.activate(Some(rel));
    }

    /// Todas las reglas CSS del documento: las de sus `<style>` y las de las hojas
    /// vinculadas (la abierta en pestaña si la hay, con lo editado; si no, la del disco).
    fn all_rule_views(&self) -> Vec<RuleView> {
        let root = self.site_root();
        let mut out = Vec::new();
        for sheet in self.doc.sheets() {
            match sheet {
                Sheet::Inline { style, css } => out.extend(rule_views_for(&css, style, None)),
                Sheet::Linked { href } => {
                    let Some(rel) = site::href_to_rel(&percent_decode(&href)) else { continue };
                    let text = match self.css_index(&rel) {
                        Some(i) => Some(self.css_files[i].text.clone()),
                        None => root
                            .as_deref()
                            .and_then(|r| site::resolve_in_root(r, &rel))
                            .and_then(|abs| std::fs::read_to_string(abs).ok()),
                    };
                    if let Some(text) = text {
                        out.extend(rule_views_for(text.trim_start_matches('\u{feff}'), 0, Some(&rel)));
                    }
                }
            }
        }
        out
    }

    /// Cambia una declaración de una regla de un `.css` externo. La hoja pasa a estar
    /// abierta (y con cambios sin guardar) como si se hubiera editado en su pestaña.
    fn set_file_decl(&mut self, rel: &str, rule: usize, name: &str, value: Option<&str>) {
        let Some(i) = self.load_css(rel) else { return };
        let text = &self.css_files[i].text;
        let edited = parse_rules(text).get(rule).and_then(|r| set_decl(text, r, name, value));
        let Some(new) = edited else {
            self.status = t("status.invalid_declaration");
            return;
        };
        self.commit_css(i, new);
        self.code_dirty = true; // si esa hoja está en pantalla, se actualiza
    }

    /// Cambia el texto de la hoja abierta `i` dejando el cambio en el historial.
    fn commit_css(&mut self, i: usize, new: String) {
        let before = std::mem::replace(&mut self.css_files[i].text, new.clone());
        self.undo_log.push(Step::Css(CssEdit { rel: self.css_files[i].rel.clone(), before, after: new }));
        self.redo_log.clear();
        self.sync_overrides();
        self.refresh_canvas_css();
    }

    /// Aplica a varios archivos el reemplazo calculado en el panel, tras confirmarlo. Lo
    /// aplicado queda sin guardar y se deshace paso a paso. Los `.html` que no son el
    /// documento abierto se omiten: no hay dónde tenerlos sin guardar.
    fn replace_in_site(&mut self, count: usize, files: Vec<SiteEdit>) {
        if files.is_empty() {
            return;
        }
        let answer = MessageDialog::new()
            .set_level(MessageLevel::Warning)
            .set_title(t("dialog.replace_in_files_title"))
            .set_description(t!("dialog.replace_in_files_text", count = count, files = files.len()))
            .set_buttons(MessageButtons::YesNo)
            .show();
        if answer != MessageDialogResult::Yes {
            self.status = t("status.replace_cancelled");
            return;
        }
        let (mut done, mut stale, mut unopened) = (0, 0, 0);
        for f in files {
            let lower = f.path.to_ascii_lowercase();
            let is_css = lower.ends_with(".css");
            if !is_css && !lower.ends_with(".html") && !lower.ends_with(".htm") {
                continue;
            }
            if is_css {
                let Some(i) = self.load_css(&f.path) else { stale += 1; continue };
                if self.css_files[i].text.encode_utf16().count() != f.from_len {
                    stale += 1;
                } else {
                    self.commit_css(i, f.text);
                    done += 1;
                }
            } else if site::rel_key(&f.path) == site::rel_key(&self.file_name()) {
                if self.doc.to_saved_html().html.encode_utf16().count() != f.from_len {
                    stale += 1;
                } else if self.run(Box::new(ReplaceDocument::new(&f.text))).is_some() {
                    self.code_force = true;
                    done += 1;
                }
            } else {
                unopened += 1;
            }
        }
        self.code_dirty = true;
        let mut msg = t!("status.replaced_in_files", n = done);
        if stale > 0 {
            msg.push_str(&t!("status.replaced_stale", n = stale));
        }
        if unopened > 0 {
            msg.push_str(&t!("status.replaced_unopened", n = unopened));
        }
        self.status = msg;
    }

    /// Fija el texto de una hoja (abriéndola si hace falta) como resultado de deshacer o
    /// rehacer; si no es lo que se esperaba, no la toca: hay ediciones posteriores que se perderían.
    fn restore_css(&mut self, edit: &CssEdit, from: &str, to: &str) -> bool {
        let Some(i) = self.load_css(&edit.rel) else { return false };
        if self.css_files[i].text != from {
            self.status = t!("status.css_changed_since", path = edit.rel);
            return false;
        }
        self.css_files[i].text = to.to_string();
        self.sync_overrides();
        self.refresh_canvas_css();
        self.code_dirty = true;
        self.code_force = true;
        true
    }

    /// Posición estructural de la selección: sobrevive a un reemplazo del documento entero.
    fn selection_key(&self) -> Option<Vec<usize>> {
        self.selection.and_then(|id| self.doc.structural_key(id))
    }

    /// Si el paso recargó el documento (los ids ya no valen), vuelve a seleccionar lo que
    /// ocupa la misma posición. Para cambios puntuales la selección ya se conserva por id.
    fn restore_selection(&mut self, patches: &[Patch], key: Option<Vec<usize>>) {
        if patches.contains(&Patch::Reload) {
            self.selection = key.and_then(|k| self.doc.node_at_key(&k));
            self.highlight();
        }
    }

    fn undo(&mut self) {
        match self.undo_log.pop() {
            None => self.status = t("status.nothing_to_undo"),
            Some(Step::Doc) => {
                let key = self.selection_key();
                if let Some((label, patches)) = self.history.undo(&mut self.doc) {
                    self.redo_log.push(Step::Doc);
                    self.after_history(&t!("status.undo", label = t(&label)), &patches);
                    self.restore_selection(&patches, key);
                }
            }
            Some(Step::Css(e)) => {
                if self.restore_css(&e, &e.after, &e.before) {
                    self.status = t!("status.undo_css", file = e.rel);
                    self.redo_log.push(Step::Css(e));
                }
            }
        }
    }

    fn redo(&mut self) {
        match self.redo_log.pop() {
            None => self.status = t("status.nothing_to_redo"),
            Some(Step::Doc) => {
                let key = self.selection_key();
                if let Some((label, patches)) = self.history.redo(&mut self.doc) {
                    self.undo_log.push(Step::Doc);
                    self.after_history(&t!("status.redo", label = t(&label)), &patches);
                    self.restore_selection(&patches, key);
                }
            }
            Some(Step::Css(e)) => {
                if self.restore_css(&e, &e.before, &e.after) {
                    self.status = t!("status.redo_css", file = e.rel);
                    self.undo_log.push(Step::Css(e));
                }
            }
        }
    }

    fn activate(&mut self, rel: Option<String>) {
        if rel.as_ref().is_some_and(|r| !self.css_files.iter().any(|c| &c.rel == r)) {
            return;
        }
        self.active_file = rel;
        self.code_dirty = true;
        self.code_force = true;
    }

    fn close_css(&mut self, rel: &str) {
        let Some(i) = self.css_files.iter().position(|c| c.rel == rel) else { return };
        if self.css_files[i].dirty() {
            self.status = t!("status.css_close_dirty", path = rel);
            return;
        }
        self.css_files.remove(i);
        self.sync_overrides();
        if self.active_file.as_deref() == Some(rel) {
            self.activate(None);
        }
    }

    fn set_file_text(&mut self, rel: &str, text: String) {
        let Some(buf) = self.css_files.iter_mut().find(|c| c.rel == rel) else { return };
        if buf.text == text {
            return;
        }
        buf.text = text;
        self.sync_overrides();
        self.refresh_canvas_css();
    }

    fn link_css(&mut self, rel: &str) {
        let html = self.doc.to_saved_html().html;
        if site::is_linked(&html, rel) {
            self.status = t!("status.css_already_linked", path = rel);
        } else if let Some(linked) = site::insert_link(&html, rel) {
            self.run(Box::new(ReplaceDocument::new(&linked)));
            self.status = t!("status.css_linked", path = rel);
            // Se muestra el HTML para que se vea la etiqueta insertada.
            self.show_code();
            self.activate(None);
        } else {
            self.status = t("status.css_no_head");
        }
    }

    /// Abre otro documento HTML de la carpeta del sitio.
    fn open_path(&mut self, rel: &str) {
        let lower = rel.to_ascii_lowercase();
        if !(lower.ends_with(".html") || lower.ends_with(".htm")) {
            self.status = t("status.html_only");
            return;
        }
        let Some(abs) = self.site_root().and_then(|r| site::resolve_in_root(&r, rel)) else {
            self.status = t!("status.cannot_open", path = rel);
            return;
        };
        if !self.confirm_discard() {
            return;
        }
        match self.load_file(&abs) {
            Ok(()) => self.status = t!("status.opened", path = abs.display()),
            Err(e) => self.status = format!("⚠ {e}"),
        }
    }

    /// Envía al panel el texto de los archivos editables del sitio (.html, .htm, .css) para
    /// buscar en ellos. Lo abierto y editado va con sus cambios sin guardar.
    fn search_site(&mut self) {
        const MAX_FILE: u64 = 1_000_000;
        const MAX_TOTAL: usize = 8_000_000;
        if self.site_files.is_empty() {
            self.refresh_files();
        }
        let cur = self.file_name();
        let mut files = vec![serde_json::json!({ "path": cur, "text": self.doc.to_saved_html().html })];
        let mut total = files[0]["text"].as_str().map_or(0, str::len);
        let mut skipped = 0usize;
        let root = self.site_root();
        for f in self.site_files.iter().filter(|f| !f.is_dir) {
            let lower = f.path.to_ascii_lowercase();
            let editable = [".html", ".htm", ".css"].iter().any(|e| lower.ends_with(e));
            if !editable || site::rel_key(&f.path) == site::rel_key(&cur) {
                continue;
            }
            let text = match self.css_index(&f.path) {
                Some(i) => Some(self.css_files[i].text.clone()),
                None => root
                    .as_deref()
                    .and_then(|r| site::resolve_in_root(r, &f.path))
                    .filter(|abs| std::fs::metadata(abs).is_ok_and(|m| m.len() <= MAX_FILE))
                    .and_then(|abs| std::fs::read_to_string(abs).ok())
                    .map(|t| t.trim_start_matches('\u{feff}').to_string()),
            };
            match text {
                Some(t) if total + t.len() <= MAX_TOTAL => {
                    total += t.len();
                    files.push(serde_json::json!({ "path": f.path, "text": t }));
                }
                _ => skipped += 1,
            }
        }
        let json = serde_json::json!({ "files": files, "skipped": skipped });
        eval(&self.popup, &format!("window.__popup && window.__popup.searchData({json})"));
    }

    fn set_arrangement(&mut self, a: Arrangement) {
        self.arrangement = a;
        self.code_open = a.code_visible();
        // Una vista que acaba de aparecer necesita el texto actual.
        self.code_dirty = self.code_open;
        self.code_force = self.code_open;
    }

    /// Muestra la vista de código; si estaba oculta, vuelve a la disposición de abajo.
    fn show_code(&mut self) {
        if self.arrangement == Arrangement::CanvasOnly {
            self.set_arrangement(Arrangement::Below);
        }
        self.code_open = true;
    }

    /// ¿Muestra la vista de código el HTML del documento, tal como lo conoce Rust?
    fn code_shows_html(&self) -> bool {
        self.code_open && self.active_file.is_none() && !self.code_dirty
    }

    // ---------- Menús desplegables y portapapeles de elementos ----------

    /// Abre el menú `name` bajo su botón. Pulsar el botón del menú abierto lo cierra: el
    /// clic en la barra ya le quitó el foco al menú (que se cerró solo), así que también
    /// cuenta como «cerrar» si el mismo menú se cerró hace un instante.
    fn open_menu(&mut self, name: String, x: f64) {
        if self.open_menu.as_deref() == Some(name.as_str()) {
            self.close_menu();
            return;
        }
        let just_closed = self
            .menu_closed
            .as_ref()
            .is_some_and(|(n, t)| *n == name && t.elapsed() < Duration::from_millis(400));
        if just_closed {
            self.menu_closed = None;
            return;
        }
        let json = serde_json::to_string(&name).expect("json");
        eval(&self.popup, &format!("window.__popup && window.__popup.open({json})"));
        eval(&self.toolbar, &format!("window.__toolbar && window.__toolbar.menu({json})"));
        self.open_menu = Some(name);
        self.menu_x = x;
        self.menu_dims = None; // el menú medirá lo que necesita nada más abrirse
        let _ = self.popup.focus();
    }

    fn close_menu(&mut self) {
        if let Some(name) = self.open_menu.take() {
            self.menu_closed = Some((name, Instant::now()));
            eval(&self.toolbar, "window.__toolbar && window.__toolbar.menu(null)");
        }
    }

    /// Guarda el texto original del elemento seleccionado para pegarlo después.
    fn copy_selection(&mut self) {
        let Some(id) = self.selection else {
            self.status = t("status.copy_select");
            return;
        };
        let tag = self.doc.tag(id).unwrap_or_default();
        if matches!(tag.as_str(), "html" | "head" | "body") {
            self.status = t!("status.copy_forbidden", tag = tag);
            return;
        }
        let html = self.doc.to_saved_html().html;
        match self.doc.outer_source(&html, id) {
            Some((text, elements)) => {
                self.clipboard = Some((text, elements));
                self.status = t!("status.copied", tag = tag);
            }
            None => self.status = t("status.element_not_located"),
        }
    }

    /// Pega lo copiado justo después del elemento seleccionado y selecciona la copia.
    fn paste_clipboard(&mut self) {
        let Some((fragment, elements)) = self.clipboard.clone() else {
            self.status = t("status.nothing_to_paste");
            return;
        };
        let Some(target) = self.selection else {
            self.status = t("status.paste_select");
            return;
        };
        self.paste_fragment(target, &fragment, elements, &t("status.pasted"));
    }

    /// Inserta `fragment` justo después de `target` (al final del cuerpo si es el `<body>`) y
    /// selecciona lo insertado. Sirve para pegar lo copiado y para los snippets: las mismas
    /// comprobaciones, el mismo deshacer.
    fn paste_fragment(&mut self, target: NodeId, fragment: &str, elements: usize, done: &str) {
        let current = self.doc.to_saved_html().html;
        let new = match self.doc.with_pasted(&current, target, fragment, elements) {
            Ok(new) => new,
            Err(e) => {
                self.status = format!("⚠ {e}");
                return;
            }
        };
        // La copia queda justo después del elemento (o al final del cuerpo): su posición es
        // la del elemento con el último índice sumado en uno.
        let is_body = self.doc.tag(target).as_deref() == Some("body");
        let mut key = self.doc.structural_key(target);
        if let Some(k) = key.as_mut().filter(|_| !is_body)
            && let Some(last) = k.last_mut()
        {
            *last += 1;
        }
        if let Some(patches) = self.run(Box::new(ReplaceDocument::new(&new))) {
            self.restore_selection(&patches, key);
            self.code_force = true;
            self.status = done.into();
        }
    }

    // ---------- Snippets ----------

    /// Los snippets que se ofrecen: primero los de serie, luego los propios. Los números que
    /// se envían a las vistas son posiciones en esta lista.
    fn all_snippets(&self) -> Vec<snippets::Snippet> {
        let mut all = snippets::builtins();
        all.extend(self.user_snippets.iter().cloned());
        all
    }

    /// Manda al menú la lista de nombres (no el HTML) y cuáles son los de serie.
    fn send_snippets(&self) {
        let list = snippet_menu(&self.all_snippets(), snippets::builtins().len());
        let json = serde_json::to_string(&list).expect("json");
        eval(&self.popup, &format!("window.__popup && window.__popup.snippets({json})"));
    }

    /// Escribe los snippets propios en disco; si falla, quedan en memoria y se avisa.
    fn persist_snippets(&mut self) {
        let Some(path) = &self.snippets_path else {
            self.status = t("status.snippets_no_storage");
            return;
        };
        if let Err(e) = snippets::save(path, &self.user_snippets) {
            self.status = t!("status.snippets_save_failed", error = e);
        }
    }

    /// Guarda el elemento seleccionado, con su texto original, como snippet propio.
    fn save_snippet(&mut self, raw_name: &str) {
        let name = match snippets::clean_name(raw_name) {
            Ok(n) => n,
            Err(e) => {
                self.status = format!("⚠ {e}");
                return;
            }
        };
        let Some(id) = self.selection else {
            self.status = t("status.snippet_select");
            return;
        };
        let tag = self.doc.tag(id).unwrap_or_default();
        if matches!(tag.as_str(), "html" | "head" | "body") {
            self.status = t!("status.snippet_root_forbidden", tag = tag);
            return;
        }
        let saved = self.doc.to_saved_html().html;
        let Some((html, elements)) = self.doc.outer_source(&saved, id) else {
            self.status = t("status.element_not_located");
            return;
        };
        if html.len() > snippets::MAX_HTML_BYTES {
            self.status = t("status.snippet_too_large");
            return;
        }
        // Que se pueda volver a insertar: lo que el parser reorganizaría fuera de su sitio
        // (una fila suelta, p. ej.) no sirve de snippet.
        if snippets::builtins().iter().any(|b| b.name.eq_ignore_ascii_case(&name)) {
            self.status = t!("status.snippet_builtin_name", name = name);
            return;
        }
        if temiktli_core::fragment_elements(&html) != elements {
            self.status = t!("status.snippet_needs_parent", tag = tag);
            return;
        }
        let replaced = match self.user_snippets.iter_mut().find(|s| s.name.eq_ignore_ascii_case(&name)) {
            Some(existing) => {
                existing.html = html;
                true
            }
            None => {
                if self.user_snippets.len() >= snippets::MAX_SNIPPETS {
                    self.status = t!("status.snippet_limit", max = snippets::MAX_SNIPPETS);
                    return;
                }
                self.user_snippets.push(snippets::Snippet { name: name.clone(), html });
                false
            }
        };
        self.persist_snippets();
        self.send_snippets();
        if !self.status.starts_with('⚠') {
            self.status = t!(if replaced { "status.snippet_updated" } else { "status.snippet_saved" }, name = name);
        }
    }

    /// Inserta el snippet `index` tras el elemento seleccionado (o al final del cuerpo).
    fn insert_snippet(&mut self, index: usize) {
        let Some(snippet) = self.all_snippets().into_iter().nth(index) else { return };
        let Some(target) = self.selection.filter(|&id| self.doc.is_attached(id)).or_else(|| self.doc.find_tag("body"))
        else {
            self.status = t("status.no_body");
            return;
        };
        let elements = temiktli_core::fragment_elements(&snippet.html);
        self.paste_fragment(target, &snippet.html, elements, &t!("status.snippet_inserted", name = snippet.name));
    }

    /// Borra un snippet propio. Los de serie no se borran.
    fn delete_snippet(&mut self, index: usize) {
        let builtin = snippets::builtins().len();
        let Some(i) = index.checked_sub(builtin).filter(|&i| i < self.user_snippets.len()) else {
            self.status = t("status.snippet_builtin_delete");
            return;
        };
        let removed = self.user_snippets.remove(i);
        self.persist_snippets();
        self.send_snippets();
        if !self.status.starts_with('⚠') {
            self.status = t!("status.snippet_deleted", name = removed.name);
        }
    }

    /// Sustituye el interior del elemento `id` por `inner` (HTML). Es como escribirlo en la
    /// vista de código: solo cambia ese trozo del texto, se puede deshacer, y el elemento
    /// sigue seleccionado aunque el lienzo se recargue.
    fn set_inner_html(&mut self, id: NodeId, inner: &str) {
        let current = self.doc.to_saved_html().html;
        let new = match self.doc.with_inner_html(&current, id, inner) {
            Ok(new) => new,
            Err(e) => {
                self.status = format!("⚠ {e}");
                return;
            }
        };
        if new == current {
            return;
        }
        let key = self.doc.structural_key(id);
        if let Some(patches) = self.run(Box::new(ReplaceDocument::new(&new))) {
            self.restore_selection(&patches, key);
            self.code_force = true;
        }
    }

    /// Limpia el HTML del elemento seleccionado, o de todo el `<body>` si no hay selección (o es
    /// `<html>`/`<head>`). Es una sola acción que se puede deshacer.
    fn clean_html(&mut self) {
        let Some(body) = self.doc.find_tag("body") else {
            self.status = t("status.no_body");
            return;
        };
        let selected = self
            .selection
            .filter(|&id| self.doc.is_attached(id))
            .filter(|&id| !matches!(self.doc.tag(id).as_deref(), Some("html" | "head")));
        let (target, whole) = match selected {
            Some(id) => (id, false),
            None => (body, true),
        };
        let current = self.doc.to_saved_html().html;
        let (new, report) = match self.doc.with_cleaned_inner(&current, target) {
            Ok(done) => done,
            Err(e) => {
                self.status = format!("⚠ {e}");
                return;
            }
        };
        let scope = t(if whole { "status.clean_scope_document" } else { "status.clean_scope_selection" });
        if report.total() == 0 {
            self.status = t!("status.clean_nothing", scope = scope);
            return;
        }
        let key = self.doc.structural_key(target);
        if let Some(patches) = self.run(Box::new(ReplaceDocument::new(&new))) {
            self.restore_selection(&patches, key);
            self.code_force = true;
            self.status = t!("status.clean_done", scope = scope, report = report.describe());
        }
    }

    /// «Indentar código» / «Reducir código»: reformatea todo el HTML del documento con `format`
    /// como una sola acción que se puede deshacer. `done` abre el mensaje de la barra de estado.
    fn reformat_html(&mut self, format: fn(&str) -> String, done: &str) {
        let current = self.doc.to_saved_html().html;
        let new = format(&current);
        if new == current {
            self.status = t!("status.reformat_nothing", done = done);
            return;
        }
        let key = self.selection_key();
        if let Some(patches) = self.run(Box::new(ReplaceDocument::new(&new))) {
            self.restore_selection(&patches, key);
            self.code_force = true;
            self.status = t!("status.reformat_done", done = done, sizes = size_summary(current.len(), new.len()));
        }
    }

    /// Revisa el documento: las reglas de HTML salen al instante y el contraste, que lo mide
    /// el lienzo, llega después en `Contrast`.
    fn start_review(&mut self) {
        self.review = Some((self.doc_version, self.doc.lint()));
        self.send_review(None);
        eval(&self.canvas, "window.__tk && window.__tk.contrast()");
    }

    /// Revisión automática tras guardar: deja el resultado en el menú Revisión y lo resume en
    /// la barra de estado. Nunca impide ni retrasa el guardado, que ya está hecho.
    fn review_after_save(&mut self) {
        self.start_review();
        if let Some(counts) = self.review_counts() {
            self.status.push_str(&t!("status.review_after_save", summary = counts.describe()));
            if counts.total() > 0 {
                self.status.push_str(&t("status.review_see_menu"));
            }
        }
    }

    /// Resultado de la última revisión, mientras siga vigente.
    fn review_counts(&self) -> Option<ReviewCounts> {
        self.review.as_ref().filter(|(v, _)| *v == self.doc_version).map(|(_, issues)| ReviewCounts::of(issues))
    }

    /// Aplica el arreglo de un clic del aviso `index` de la última revisión y vuelve a revisar, para
    /// que la lista refleje el resultado. Cuenta como cualquier otra edición: se puede deshacer.
    fn fix_issue(&mut self, index: usize) {
        let fix = match &self.review {
            Some((v, issues)) if *v == self.doc_version => issues.get(index).and_then(|i| i.fix.clone()),
            // Los ids de la revisión ya no valen: aplicar un arreglo podría tocar otro elemento.
            _ => {
                self.status = t("status.review_stale");
                return;
            }
        };
        let Some(fix) = fix else { return };

        let applied = match &fix.action {
            FixAction::SetAttr { id, name, value } => {
                self.run(Box::new(SetAttr::new(*id, name.clone(), value.clone()))).is_some()
            }
            FixAction::InsertInHead { .. } | FixAction::AddDoctype => {
                let current = self.doc.to_saved_html().html;
                let edited = match &fix.action {
                    FixAction::InsertInHead { html } => self.doc.with_head_prepended(&current, html),
                    _ => self.doc.with_doctype(&current),
                };
                match edited {
                    Ok(new) => {
                        // Los ids cambian al reemplazar el texto: la selección se conserva por posición.
                        let key = self.selection_key();
                        match self.run(Box::new(ReplaceDocument::new(&new))) {
                            Some(patches) => {
                                self.restore_selection(&patches, key);
                                self.code_force = true;
                                true
                            }
                            None => false,
                        }
                    }
                    Err(e) => {
                        self.status = format!("⚠ {e}");
                        false
                    }
                }
            }
        };
        if applied {
            self.status = t!("status.fixed", label = fix.label);
            self.start_review();
        }
    }

    /// Añade a la revisión los textos de poco contraste que midió el lienzo.
    fn finish_review(&mut self, items: Vec<ContrastItem>, checked: usize, skipped: usize) {
        let version = self.doc_version;
        let Some((_, issues)) = self.review.as_mut().filter(|(v, _)| *v == version) else {
            return; // llegó tarde: el documento ya cambió
        };
        for it in items {
            let (min, size) = if it.large { (3.0, t("lint.contrast_size_large")) } else { (4.5, t("lint.contrast_size_normal")) };
            issues.push(Issue {
                id: Some(it.id),
                severity: Severity::Warning,
                category: Category::Accessibility,
                rule: "contrast".into(),
                message: t!("lint.contrast", ratio = it.ratio, min = min, size = size, fg = it.fg, bg = it.bg),
                // Mejorar el contraste es una decisión de diseño (qué color usar): sin arreglo mecánico.
                fix: None,
            });
        }
        issues.sort_by_key(|i| (i.severity, i.id.unwrap_or(0)));
        self.send_review(Some((checked, skipped)));
    }

    /// Envía la revisión al panel. `contrast` es `None` mientras el lienzo aún mide.
    fn send_review(&self, contrast: Option<(usize, usize)>) {
        let Some((_, issues)) = &self.review else { return };
        let json = serde_json::json!({
            "issues": issues,
            "contrast": contrast.map(|(checked, skipped)| serde_json::json!({ "checked": checked, "skipped": skipped })),
        });
        eval(&self.popup, &format!("window.__popup && window.__popup.review({json})"));
    }

    /// Si el documento cambió desde la revisión, sus ids ya no valen: se avisa al panel.
    fn expire_review(&mut self) {
        if self.review.as_ref().is_some_and(|(v, _)| *v != self.doc_version) {
            self.review = None;
            eval(&self.popup, "window.__popup && window.__popup.reviewStale()");
        }
    }

    /// Mapa de posiciones de `code_text`, reutilizado mientras el documento y el texto no cambien.
    fn source_index(&mut self) -> Option<&SourceIndex> {
        let key = (self.doc_version, self.code_text.len());
        if self.span_index.as_ref().is_none_or(|c| (c.0, c.1) != key) {
            let ix = self.doc.source_index(&self.code_text);
            self.span_index = Some((key.0, key.1, ix));
        }
        self.span_index.as_ref().and_then(|c| c.2.as_ref())
    }

    /// Marca en la vista de código la etiqueta de inicio del elemento seleccionado en el lienzo.
    fn show_in_code(&mut self, id: NodeId) {
        if !self.code_shows_html() {
            return;
        }
        let Some(r) = self.source_index().and_then(|ix| ix.tag_range(id)) else { return };
        let (line, col) = line_col_utf16(&self.code_text, r.start);
        let len = self.code_text[r.clone()].encode_utf16().count();
        eval(&self.code, &format!("window.__code && window.__code.show({line}, {col}, {len})"));
    }

    /// El cursor de la vista de código cayó en `offset` (UTF-16): se selecciona ese elemento.
    fn select_from_cursor(&mut self, offset: usize) {
        if !self.code_shows_html() {
            return;
        }
        let pos = utf16_to_byte(&self.code_text, offset);
        if let Some(id) = self.source_index().and_then(|ix| ix.element_at(pos))
            && self.selection != Some(id)
        {
            self.selection = Some(id);
            self.status.clear();
            self.highlight();
            eval(&self.canvas, &format!("window.__tk && window.__tk.reveal({id})"));
        }
    }

    /// Abre `rel` (.html o .css) en la vista de código y selecciona la coincidencia.
    fn go_to(&mut self, rel: &str, line: usize, col: usize, len: usize) {
        let lower = rel.to_ascii_lowercase();
        if lower.ends_with(".css") {
            self.open_css(rel);
            if self.css_index(rel).is_none() {
                return;
            }
        } else if site::rel_key(rel) != site::rel_key(&self.file_name()) {
            self.open_path(rel);
            if site::rel_key(rel) != site::rel_key(&self.file_name()) {
                return; // cancelado o no se pudo abrir
            }
            self.show_code();
            self.activate(None);
        } else {
            self.show_code();
            self.activate(None);
        }
        self.pending_goto = Some((line, col, len));
    }

    /// Pestañas de la vista de código (solo se redibujan si cambian).
    fn push_tabs(&self) {
        let tabs = serde_json::json!({
            "main": { "name": self.file_name(), "dirty": self.dirty },
            "files": self.css_files.iter().map(|c| serde_json::json!({ "path": c.rel, "dirty": c.dirty() })).collect::<Vec<_>>(),
            "active": self.active_file,
        });
        eval(&self.code, &format!("window.__code && window.__code.tabs({tabs})"));
    }

    /// Lleva al lienzo el resultado de un comando: parches puntuales, o recarga completa
    /// si el documento entero cambió (la selección deja de ser válida).
    fn project(&mut self, patches: &[Patch]) {
        if patches.contains(&Patch::Reload) {
            self.selection = None;
            self.sync_shared();
            self.reload_canvas();
        } else {
            for p in patches {
                self.send_patch(p);
            }
            // El documento cambió (insertar, borrar, deshacer…): el destino pudo moverse.
            self.send_insert_mark();
        }
    }

    /// Envía el HTML actual a la vista de código si cambió desde el último envío.
    fn push_code(&mut self) {
        if !self.code_open || !self.code_dirty {
            return;
        }
        // Lo que se muestra es la pestaña activa: una hoja de estilo o el documento.
        let (text, kind) = match self.active_file.as_ref().and_then(|a| self.css_files.iter().find(|c| &c.rel == a)) {
            Some(css) => (css.text.clone(), "css"),
            None => {
                self.code_text = self.doc.to_saved_html().html;
                (self.code_text.clone(), "html")
            }
        };
        let json = serde_json::to_string(&text).expect("json");
        eval(
            &self.code,
            &format!("window.__code && window.__code.set({json}, {}, '{kind}')", self.code_force),
        );
        self.code_dirty = false;
        self.code_force = false;
        if let Some((line, col, len)) = self.pending_goto.take() {
            eval(&self.code, &format!("window.__code && window.__code.goto({line}, {col}, {len})"));
        }
    }

    fn send_patch(&self, patch: &Patch) {
        let json = serde_json::to_string(patch).expect("json");
        eval(&self.canvas, &format!("window.__tk && window.__tk.apply({json})"));
    }

    fn highlight(&self) {
        self.send_patch(&Patch::Highlight { id: self.selection });
        self.send_insert_mark();
    }

    /// Arrastrando `id` sobre `target`: si se puede soltar en `place`, el cursor de inserción
    /// marca dónde caería; si no, el lienzo muestra el cursor de «prohibido» y el motivo.
    fn drag_over(&mut self, id: NodeId, target: NodeId, place: &str) {
        match self.doc.resolve_drop(id, target, place) {
            Ok((parent, index)) => {
                self.send_patch(&Patch::InsertMark { parent: Some(parent), index });
                self.send_patch(&Patch::DropState { ok: true, why: None });
            }
            Err(why) => {
                self.send_patch(&Patch::InsertMark { parent: None, index: 0 });
                self.send_patch(&Patch::DropState { ok: false, why: Some(why) });
            }
        }
    }

    /// Soltó `id` en `place` de `target`. Si las reglas de anidación no lo permiten, no se toca
    /// nada: el elemento sigue donde estaba y la barra de estado dice por qué.
    fn move_element(&mut self, id: NodeId, target: NodeId, place: &str) {
        match self.doc.resolve_drop(id, target, place) {
            Ok((parent, index)) => {
                let same_place =
                    self.doc.parent_and_index(id).is_some_and(|(p, i)| p == parent && (index == i || index == i + 1));
                if same_place {
                    self.status = t("status.moved_same_place");
                } else if self.run(Box::new(MoveNode::new(id, parent, index))).is_some() {
                    self.selection = Some(id);
                    self.status = t!("status.moved", path = self.doc.path(id).unwrap_or_default());
                }
            }
            Err(why) => self.status = t!("status.move_rejected", why = why),
        }
        self.highlight();
    }

    /// Pinta en el lienzo dónde caerá el próximo elemento.
    fn send_insert_mark(&self) {
        self.send_patch(&insert_mark(&self.doc, self.selection));
    }

    /// Ejecuta un comando y proyecta sus parches al lienzo.
    fn run(&mut self, cmd: Box<dyn Command>) -> Option<Vec<Patch>> {
        match self.history.execute(&mut self.doc, cmd) {
            Ok(patches) => {
                self.doc_version += 1;
                self.undo_log.push(Step::Doc);
                self.redo_log.clear();
                self.project(&patches);
                self.dirty = true;
                self.code_dirty = true;
                self.status.clear();
                Some(patches)
            }
            Err(e) => {
                self.status = format!("⚠ {e}");
                None
            }
        }
    }

    fn handle(&mut self, msg: FromWebview) {
        let is_matches = matches!(msg, FromWebview::Matches { .. });
        // Lo único que escribe el usuario en la vista de código; todo lo demás viene de fuera.
        let from_code = matches!(msg, FromWebview::SetSource { .. } | FromWebview::SetFileText { .. });
        match msg {
            FromWebview::Ready => {
                self.highlight();
                // Cualquier vista que termina de cargar necesita el código actual.
                self.code_dirty = true;
                self.code_force = true;
                self.files_dirty = true;
                self.send_snippets();
            }
            FromWebview::InsertImage { path } => self.insert_image(&path),
            FromWebview::NewFile { path } => self.new_file(&path),
            FromWebview::OpenPath { path } => self.open_path(&path),
            FromWebview::OpenCss { path } => self.open_css(&path),
            FromWebview::ActivateFile { path } => self.activate(path),
            FromWebview::CloseFile { path } => self.close_css(&path),
            FromWebview::SetFileText { path, text } => self.set_file_text(&path, text),
            FromWebview::LinkCss { path } => self.link_css(&path),
            FromWebview::DeletePath { path } => self.delete_path(&path),
            FromWebview::RenamePath { path, new_name } => self.rename_path(&path, &new_name),
            FromWebview::RefreshFiles => self.refresh_files(),
            FromWebview::SearchSite => self.search_site(),
            FromWebview::ReplaceInSite { count, files } => self.replace_in_site(count, files),
            FromWebview::GoTo { path, line, col, len } => {
                self.close_menu(); // el resultado se ve en el código: el menú estorbaría
                self.go_to(&path, line, col, len);
            }
            FromWebview::SetSource { html } => {
                // Lo que ya muestra la vista de código no es un cambio.
                if html != self.code_text {
                    // Los ids cambian al reemplazar el documento: la selección se conserva
                    // por posición para que el lienzo no «olvide» lo que se estaba mirando.
                    let key = self.selection_key();
                    if let Some(patches) = self.run(Box::new(ReplaceDocument::new(&html))) {
                        self.restore_selection(&patches, key);
                    }
                    self.code_text = html;
                    // El texto escrito es ya el de la vista: no hace falta devolvérselo.
                    self.code_dirty = false;
                }
            }
            FromWebview::SetFileDecl { path, rule, name, value } => {
                self.set_file_decl(&path, rule, &name, value.as_deref());
            }
            FromWebview::SetCodeView { open } => {
                if open {
                    self.show_code();
                } else {
                    self.set_arrangement(Arrangement::CanvasOnly);
                }
            }
            FromWebview::SetLayout { layout } => match Arrangement::parse(&layout) {
                Some(a) => self.set_arrangement(a),
                None => self.status = t!("status.layout_unsupported", layout = layout),
            },
            FromWebview::Select { id } => {
                if self.doc.contains(id) {
                    self.close_menu(); // si viene de un aviso de Revisión, se ve el elemento
                    self.selection = Some(id);
                    self.status.clear();
                    self.highlight();
                    self.show_in_code(id);
                    // Viene del panel (un aviso de la revisión) o del lienzo: si no se ve, se trae.
                    eval(&self.canvas, &format!("window.__tk && window.__tk.reveal({id})"));
                }
            }
            FromWebview::CursorAt { offset } => self.select_from_cursor(offset),
            FromWebview::Copy => self.copy_selection(),
            FromWebview::Paste => self.paste_clipboard(),
            FromWebview::OpenMenu { name, x } => self.open_menu(name, x),
            FromWebview::CloseMenu => self.close_menu(),
            FromWebview::MenuSize { width, height } => {
                // Un tamaño absurdo (o NaN) no debe descolocar la ventana.
                if width.is_finite() && height.is_finite() && width >= 40.0 && height >= 20.0 {
                    self.menu_dims = Some((width.min(900.0), height.min(900.0)));
                }
            }
            FromWebview::SetInner { id, html } => self.set_inner_html(id, &html),
            FromWebview::Review => self.start_review(),
            FromWebview::FixIssue { index } => self.fix_issue(index),
            FromWebview::CleanHtml => {
                self.close_menu();
                self.clean_html();
            }
            FromWebview::IndentHtml => {
                self.close_menu();
                self.reformat_html(temiktli_core::indent_html, &t("status.indent_done"));
            }
            FromWebview::MinifyHtml => {
                self.close_menu();
                self.reformat_html(temiktli_core::minify_html, &t("status.minify_done"));
            }
            FromWebview::SetLanguage { lang } => {
                self.close_menu();
                self.set_language(lang);
            }
            FromWebview::SetAutoReview { on } => self.auto_review = on,
            FromWebview::SaveSnippet { name } => {
                self.close_menu();
                self.save_snippet(&name);
            }
            FromWebview::InsertSnippet { index } => {
                self.close_menu();
                self.insert_snippet(index);
            }
            FromWebview::DeleteSnippet { index } => self.delete_snippet(index),
            FromWebview::Contrast { items, checked, skipped } => self.finish_review(items, checked, skipped),            FromWebview::EditText { id, text } => {
                if self.doc.text(id).as_deref() != Some(text.as_str())
                    && self.run(Box::new(SetText::new(id, text))).is_none()
                {
                    // El lienzo ya muestra el texto editado: devolverle el real.
                    if let Some(real) = self.doc.text(id) {
                        self.send_patch(&Patch::SetText { id, text: real });
                    }
                }
            }
            FromWebview::SetAttr { id, name, value } => {
                if self.doc.attr(id, &name.to_ascii_lowercase()) != value {
                    self.run(Box::new(SetAttr::new(id, name, value)));
                }
            }
            FromWebview::Insert { tag } => self.insert(tag),
            FromWebview::Delete => match self.selection {
                Some(id) => {
                    if self.run(Box::new(DeleteNode::new(id))).is_some() {
                        self.selection = None;
                        self.highlight();
                    }
                }
                None => self.status = t("status.delete_select"),
            },
            FromWebview::DragOver { id, target, place } => self.drag_over(id, target, &place),
            FromWebview::MoveElement { id, target, place } => self.move_element(id, target, &place),
            FromWebview::DragEnd => self.send_insert_mark(),
            FromWebview::Undo => self.undo(),
            FromWebview::Redo => self.redo(),
            FromWebview::Open => self.open_dialog(),
            FromWebview::Save => {
                self.save(false);
            }
            FromWebview::SaveAs => {
                self.save(true);
            }
            FromWebview::CloseDocument => self.close_document(),
            FromWebview::SetViewport { width } => match width {
                None => self.viewport = None,
                Some(w) if VIEWPORT_PRESETS.contains(&w) => self.viewport = Some(w),
                Some(w) => self.status = t!("status.viewport_unsupported", width = w),
            },
            FromWebview::SetTheme { theme } => match theme.as_str() {
                "system" | "light" | "dark" => {
                    self.theme = theme;
                    self.apply_theme();
                }
                other => self.status = t!("status.theme_unknown", theme = other),
            },
            FromWebview::SetDecl { style, rule, name, value } => {
                match self.doc.css_with_decl(style, rule, &name, value.as_deref()) {
                    Some(css) => {
                        self.run(Box::new(SetText::new(style, css)));
                    }
                    None => self.status = t("status.invalid_declaration"),
                }
            }
            FromWebview::Matches { id, rules } => {
                // Una respuesta tardía (otra selección, o CSS ya cambiado) se descarta.
                if Some(id) == self.selection && rules.len() == self.probed_rules.len() {
                    self.matched_rules = self
                        .probed_rules
                        .iter()
                        .zip(rules)
                        .filter_map(|(r, hit)| hit.then(|| r.clone()))
                        .collect();
                }
            }
        }
        if !is_matches {
            self.request_matches();
        }
        // Un cambio que no nació en el editor de código se muestra siempre: el usuario no
        // está escribiendo ahí (aunque ese webview aún crea tener el foco).
        if !from_code && self.code_dirty {
            self.code_force = true;
        }
        self.push_tabs();
        self.push_code();
        self.push_state();
        self.expire_review();
    }

    /// Pide al lienzo qué reglas CSS afectan al elemento seleccionado; la respuesta llega
    /// como `Matches`.
    fn request_matches(&mut self) {
        let Some(id) = self.selection else {
            self.probed_rules.clear();
            self.matched_rules.clear();
            return;
        };
        self.probed_rules = self.all_rule_views();
        let probe: Vec<_> = self
            .probed_rules
            .iter()
            .map(|r| serde_json::json!({ "selector": r.selector, "context": r.context }))
            .collect();
        let json = serde_json::to_string(&probe).expect("json");
        eval(&self.canvas, &format!("window.__tk && window.__tk.matchRules({id}, {json})"));
    }

    fn relayout(&mut self, window: &Window) {
        let size = window.inner_size();
        let z = boxes(
            (size.width, size.height),
            window.scale_factor(),
            self.viewport,
            self.arrangement,
            self.fractions,
        );
        let vis = (self.arrangement.canvas_visible(), self.code_open);
        let popup = self
            .open_menu
            .as_deref()
            .map(|name| popup_px((size.width, size.height), window.scale_factor(), name, self.menu_x, self.menu_dims));
        if self.last_layout == Some((z, vis, popup)) {
            return;
        }
        self.last_layout = Some((z, vis, popup));
        let _ = self.toolbar.set_bounds(rect(z.toolbar.0, z.toolbar.1, z.toolbar.2, z.toolbar.3));
        if let Some(p) = popup {
            let _ = self.popup.set_bounds(rect(p.0, p.1, p.2, p.3));
        }
        let _ = self.popup.set_visible(popup.is_some());
        let _ = self.canvas.set_bounds(rect(z.canvas.0, z.canvas.1, z.canvas.2, z.canvas.3));
        let _ = self.canvas.set_visible(vis.0);
        let _ = self.code.set_bounds(rect(z.code.0, z.code.1, z.code.2, z.code.3));
        let _ = self.code.set_visible(vis.1);
        let _ = self.panel.set_bounds(rect(z.panel.0, z.panel.1, z.panel.2, z.panel.3));
        // Lo que dejó libre un webview al encogerse no se borra solo.
        paint_window_background(window);
    }

    /// El ratón se movió sobre la ventana principal (solo llega desde lo que no cubre un
    /// webview, o desde cualquier sitio mientras se arrastra con el ratón capturado).
    fn pointer_moved(&mut self, window: &Window, pos: (f64, f64)) {
        let size = (window.inner_size().width, window.inner_size().height);
        let scale = window.scale_factor();
        if self.dragging {
            if let Some(f) = fraction_at(size, scale, self.arrangement, pos) {
                match self.arrangement {
                    Arrangement::Below => self.fractions.below = f,
                    Arrangement::Side => self.fractions.side = f,
                    Arrangement::CodeOnly | Arrangement::CanvasOnly => {}
                }
                self.relayout(window);
            }
            return;
        }
        let z = boxes(size, scale, self.viewport, self.arrangement, self.fractions);
        let over = z.divider.is_some_and(|d| inside(d, pos));
        if over != self.over_divider {
            self.over_divider = over;
            window.set_cursor_icon(match (over, self.arrangement) {
                (true, Arrangement::Side) => CursorIcon::ColResize,
                (true, _) => CursorIcon::RowResize,
                (false, _) => CursorIcon::Default,
            });
        }
    }

    /// Botón izquierdo del ratón: pulsar sobre el divisor empieza a arrastrarlo.
    fn pointer_button(&mut self, window: &Window, pressed: bool) {
        if pressed && self.over_divider {
            self.dragging = true;
            capture_mouse(window, true);
        } else if !pressed && self.dragging {
            self.dragging = false;
            capture_mouse(window, false);
        }
    }

    /// Fuerza el `prefers-color-scheme` del lienzo (solo el lienzo: el panel sigue al sistema).
    fn apply_theme(&mut self) {
        #[cfg(windows)]
        {
            use wry::{Theme, WebViewExtWindows};
            let t = match self.theme.as_str() {
                "light" => Theme::Light,
                "dark" => Theme::Dark,
                _ => Theme::Auto,
            };
            if let Err(e) = self.canvas.set_theme(t) {
                self.status = t!("status.theme_failed", error = e);
            }
        }
    }

    /// Guarda el idioma elegido en el menú («system» o un código). Se aplica al reiniciar: los
    /// webviews ya tienen sus textos incrustados y el núcleo su diccionario.
    fn set_language(&mut self, lang: String) {
        if lang != "system" && !i18n::is_available(&lang) {
            return;
        }
        let name = if lang == "system" {
            t("menu.view.language_system")
        } else {
            i18n::language_names().into_iter().find(|(c, _)| *c == lang).map(|(_, n)| n).unwrap_or_default()
        };
        let Some(path) = &self.config_path else {
            self.status = t("status.language_no_storage");
            return;
        };
        let mut cfg = config::load(path);
        cfg.language = Some(lang.clone());
        match config::save(path, &cfg) {
            Ok(()) => {
                self.language = lang;
                self.status = t!("status.language_saved", name = name);
                // La barra de estado pasa desapercibida y el cambio no se ve hasta reiniciar:
                // un diálogo deja claro que hay que hacerlo.
                MessageDialog::new()
                    .set_level(MessageLevel::Info)
                    .set_title(t("menu.view.language"))
                    .set_description(self.status.clone())
                    .set_buttons(MessageButtons::Ok)
                    .show();
            }
            Err(e) => self.status = t!("status.language_save_failed", error = e),
        }
    }

    fn after_history(&mut self, status: &str, patches: &[Patch]) {
        self.doc_version += 1;
        self.project(patches);
        self.dirty = true;
        // Deshacer/rehacer puede cambiar el código por debajo del cursor: se fuerza.
        self.code_dirty = true;
        self.code_force = true;
        self.status = status.into();
        // Deshacer una inserción deja la selección apuntando a un nodo descolgado.
        if self.selection.is_some_and(|id| !self.doc.is_attached(id)) {
            self.selection = None;
        }
        self.highlight();
    }

    /// Dónde cae un elemento nuevo según la selección, al estilo de un panel Insertar.
    fn insert(&mut self, tag: String) {
        self.insert_with(tag, Vec::new());
    }

    /// Inserta una imagen del sitio como `<img src alt>`, donde caería cualquier elemento nuevo.
    fn insert_image(&mut self, rel: &str) {
        let Some(root) = self.site_root() else {
            self.status = t("status.image_no_folder");
            return;
        };
        if !site::is_image(rel) || site::resolve_in_root(&root, rel).is_none() {
            self.status = t!("status.image_cannot_insert", path = rel);
            return;
        }
        let attrs = vec![
            ("src".to_string(), site::image_src(rel)),
            ("alt".to_string(), site::alt_from(rel)),
        ];
        self.insert_with("img".into(), attrs);
        self.status = t!("status.image_inserted", path = rel);
    }

    /// Crea una carpeta (si lo escrito termina en `/` o no tiene extensión) o un archivo nuevo
    /// en la carpeta del sitio, sin pisar nada. Un archivo creado se abre.
    fn new_file(&mut self, rel: &str) {
        let rel = rel.trim();
        let Some(root) = self.site_root() else {
            self.status = t("status.new_no_folder");
            return;
        };
        if site::wants_folder(rel) {
            let shown = rel.trim_end_matches('/');
            match site::new_folder_target(&root, rel).and_then(|dir| {
                std::fs::create_dir_all(&dir).map_err(|e| t!("status.folder_create_failed", path = shown, error = e))
            }) {
                Ok(()) => {
                    self.refresh_files();
                    self.status = t!("status.folder_created", path = shown);
                }
                Err(e) => self.status = format!("⚠ {e}"),
            }
            return;
        }
        let target = match site::new_file_target(&root, rel) {
            Ok(t) => t,
            Err(e) => {
                self.status = format!("⚠ {e}");
                return;
            }
        };
        // `create_new` falla si el archivo apareció entretanto: nunca se sobrescribe.
        let created = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .and_then(|mut f| std::io::Write::write_all(&mut f, site::new_file_template(rel).as_bytes()));
        if let Err(e) = created {
            self.status = t!("status.file_create_failed", path = rel, error = e);
            return;
        }
        self.refresh_files();
        if rel.to_ascii_lowercase().ends_with(".css") {
            self.open_css(rel);
        } else {
            self.open_path(rel);
        }
        if !self.status.starts_with('⚠') {
            self.status = t!("status.file_created", path = rel);
        }
    }

    /// Ruta del documento abierto relativa a la carpeta del sitio (con `/`), o `None` si no
    /// tiene ruta o no está dentro de `root`. Se canonicaliza para que coincida con las rutas
    /// del panel aunque Windows las escriba distinto.
    fn open_rel(&self, root: &Path) -> Option<String> {
        rel_within(root, self.path.as_deref()?)
    }

    /// Envía a la Papelera un archivo o carpeta del sitio tras confirmarlo (con el detalle de
    /// lo que contiene y de los enlaces que se romperán). Si es el documento abierto, o lo
    /// contiene, el documento sigue abierto como uno sin ruta, con sus cambios.
    fn delete_path(&mut self, rel: &str) {
        let Some(root) = self.site_root() else {
            self.status = t("status.delete_no_folder");
            return;
        };
        let abs = match fileops::locate(&root, rel) {
            Ok(a) => a,
            Err(e) => {
                self.status = format!("⚠ {e}");
                return;
            }
        };
        let is_dir = abs.is_dir();
        // Hay que medirlo antes de borrar: después ya no se puede canonicalizar la ruta.
        let open_rel = self.open_rel(&root);
        let im = fileops::impact(&root, rel, is_dir, true);
        let dirty_css =
            self.css_files.iter().filter(|c| c.dirty() && relink::points_into(&c.rel, rel, is_dir)).count();
        let answer = MessageDialog::new()
            .set_level(MessageLevel::Warning)
            .set_title(t("dialog.delete_title"))
            .set_description(delete_summary(rel, is_dir, &im, dirty_css))
            .set_buttons(MessageButtons::OkCancel)
            .show();
        if answer != MessageDialogResult::Ok {
            return;
        }
        if let Err(e) = fileops::delete_to_trash(&root, rel, &fileops::SystemTrash) {
            self.status = format!("⚠ {e}");
            return;
        }
        // Las hojas abiertas dentro de lo borrado se cierran (sus pasos de deshacer, también).
        let gone = |r: &str| relink::points_into(r, rel, is_dir);
        self.css_files.retain(|c| !gone(&c.rel));
        let keep = |s: &Step| !matches!(s, Step::Css(e) if gone(&e.rel));
        self.undo_log.retain(keep);
        self.redo_log.retain(keep);
        self.sync_overrides();
        self.refresh_canvas_css();
        if self.active_file.as_deref().is_some_and(gone) {
            self.activate(None);
        }
        // El documento abierto queda sin ruta: sigue abierto, con sus cambios, y «Guardar»
        // pide dónde guardarlo (el archivo borrado no se recrea solo).
        if open_rel.as_deref().is_some_and(gone) {
            self.path = None;
            self.dirty = true;
            self.code_dirty = true;
            self.code_force = true;
            self.sync_shared();
            self.reload_canvas();
        }
        self.refresh_files();
        self.status = t!("status.sent_to_trash", path = rel);
    }

    /// Renombra un archivo o carpeta del sitio. Pide confirmación si cambia la extensión, si
    /// reemplaza algo y —decidiendo el usuario— si otros archivos enlazan al elemento. Lo que
    /// está abierto sigue abierto con su ruta nueva; los enlaces de los archivos abiertos se
    /// actualizan en memoria (no en disco) para no pisar su trabajo.
    fn rename_path(&mut self, rel: &str, new_name: &str) {
        let Some(root) = self.site_root() else {
            self.status = t("status.rename_no_folder");
            return;
        };
        let plan = match fileops::plan_rename(&root, rel, new_name) {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("⚠ {e}");
                return;
            }
        };
        if plan.unchanged {
            self.status = t("status.rename_unchanged");
            return;
        }
        let ext_of = |n: &str| n.rsplit_once('.').map_or(t("dialog.extension_none"), |(_, e)| format!(".{e}"));
        let current = plan.from_rel.rsplit('/').next().unwrap_or(&plan.from_rel);
        if plan.ext_changed
            && !confirm(
                &t("dialog.extension_title"),
                t!("dialog.extension_text", name = plan.from_rel, from = ext_of(current), to = ext_of(new_name)),
            )
        {
            return;
        }
        if plan.exists {
            // Las hojas abiertas dentro de lo reemplazado se cierran: que el usuario lo sepa.
            let dirty_css = self
                .css_files
                .iter()
                .filter(|c| c.dirty() && relink::points_into(&c.rel, &plan.to_rel, plan.exists_is_dir))
                .count();
            if !confirm(&t("dialog.replace_title"), replace_text(&plan, dirty_css)) {
                return;
            }
        }
        let im = fileops::impact(&root, &plan.from_rel, plan.is_dir, false);
        let mut update_links = false;
        if im.referrers.is_empty() && im.not_checked > 0 {
            // Nada que actualizar, pero no todo se pudo revisar: no se renombra sin avisar.
            if !confirm(&t("dialog.rename_title"), unchecked_text(&plan, im.not_checked)) {
                return;
            }
        }
        if !im.referrers.is_empty() {
            let answer = MessageDialog::new()
                .set_level(MessageLevel::Warning)
                .set_title(t("dialog.rename_title"))
                .set_description(rename_summary(&plan, &im))
                .set_buttons(MessageButtons::YesNoCancel)
                .show();
            match answer {
                MessageDialogResult::Yes => update_links = true,
                MessageDialogResult::No => {}
                _ => return,
            }
        }
        // Antes de mover nada: dónde está el documento abierto, relativo al sitio.
        let doc_old = self.open_rel(&root);
        if let Err(e) = fileops::apply_rename(&plan, plan.exists, &fileops::SystemTrash) {
            self.status = format!("⚠ {e}");
            return;
        }
        let (from, to, is_dir) = (plan.from_rel.as_str(), plan.to_rel.as_str(), plan.is_dir);

        // Lo que estaba abierto y fue reemplazado (ya está en la Papelera) se cierra; el
        // documento queda sin ruta, con sus cambios.
        let replaced = |r: &str| plan.exists && relink::points_into(r, to, plan.exists_is_dir);
        let active_replaced = self.active_file.as_deref().is_some_and(replaced);
        self.css_files.retain(|c| !replaced(&c.rel));
        let keep = |s: &Step| !matches!(s, Step::Css(e) if replaced(&e.rel));
        self.undo_log.retain(keep);
        self.redo_log.retain(keep);
        if active_replaced {
            self.activate(None);
        }
        let mut doc_now = doc_old.clone();
        if doc_old.as_deref().is_some_and(replaced) {
            self.path = None;
            self.dirty = true;
            doc_now = None;
        } else if let Some(new_rel) = doc_old.as_deref().and_then(|o| rename_open_path(o, from, to, is_dir)) {
            self.path = Some(new_rel.split('/').fold(root.clone(), |p, seg| p.join(seg)));
            doc_now = Some(new_rel);
        }
        // Rutas nuevas de las hojas abiertas (y la pestaña activa).
        let mut moved: Vec<(String, String)> = Vec::new();
        for c in &mut self.css_files {
            if let Some(n) = rename_open_path(&c.rel, from, to, is_dir) {
                moved.push((c.rel.clone(), n.clone()));
                if let Some(abs) = site::resolve_in_root(&root, &n) {
                    c.abs = abs;
                }
                c.rel = n;
            }
        }
        if let Some(n) = self.active_file.as_deref().and_then(|a| rename_open_path(a, from, to, is_dir)) {
            self.active_file = Some(n);
        }
        // Deshacer / rehacer deben apuntar a la hoja con su nombre nuevo.
        remap_steps(&mut self.undo_log, from, to, is_dir);
        remap_steps(&mut self.redo_log, from, to, is_dir);

        let mut report = fileops::RelinkReport::default();
        let mut in_memory = 0;
        if update_links {
            // Lo abierto se actualiza en memoria; el resto, en disco.
            let mut skip: Vec<String> = self.css_files.iter().map(|c| c.rel.clone()).collect();
            skip.extend(doc_now.clone());
            report = fileops::relink_site(&root, &plan, &skip);
            for c in &mut self.css_files {
                let old = moved.iter().find(|(_, n)| *n == c.rel).map_or(c.rel.clone(), |(o, _)| o.clone());
                let (text, n) = relink::retarget(&c.text, relink::Kind::Css, &old, &c.rel, from, to, is_dir);
                if n > 0 {
                    c.text = text;
                    in_memory += n;
                }
            }
            if let Some(now) = &doc_now {
                let old = doc_old.clone().unwrap_or_else(|| now.clone());
                let html = self.doc.to_saved_html().html;
                let (new_html, n) = relink::retarget(&html, relink::Kind::Html, &old, now, from, to, is_dir);
                if n > 0 {
                    self.run(Box::new(ReplaceDocument::new(&new_html)));
                    in_memory += n;
                }
            }
        }

        self.sync_overrides();
        self.sync_shared();
        self.refresh_canvas_css();
        self.reload_canvas();
        self.code_dirty = true;
        self.code_force = true;
        self.refresh_files();
        self.status = t!("status.renamed", from = from, to = to);
        if update_links {
            let done = report.updated.len() + usize::from(in_memory > 0);
            if done > 0 {
                self.status.push_str(&t!("status.renamed_links_updated", files = tn("units.file", done)));
            }
            if !report.failed.is_empty() {
                let names: Vec<_> = report.failed.iter().map(|(r, e)| format!("{r}: {e}")).collect();
                self.status.push_str(&t!("status.renamed_links_failed", list = names.join("; ")));
            }
        }
    }

    fn insert_with(&mut self, tag: String, attrs: Vec<(String, String)>) {
        // Dónde cae lo decide el núcleo según lo seleccionado (una celda en una fila, un
        // <title> en el <head>…); si no hay sitio válido, dice qué hay que seleccionar.
        let (parent, index) = match self.doc.insertion_point(&tag, self.selection) {
            Ok(spot) => spot,
            Err(why) => {
                self.status = format!("⚠ {why}");
                return;
            }
        };

        if let Some(patches) = self.run(Box::new(InsertElement::with_attrs(parent, index, tag, attrs))) {
            if let Some(Patch::Insert { parent, index, .. }) = patches.first() {
                if let Some(id) = self.doc.child_at(*parent, *index) {
                    self.selection = Some(id);
                    self.highlight();
                }
            }
        }
    }

    // ---------- Archivos ----------

    fn sync_shared(&self) {
        let mut s = self.shared.lock().expect("estado compartido");
        s.canvas_html = self.doc.to_canvas_html();
        s.site_root = self.path.as_deref().and_then(Path::parent).map(Path::to_path_buf);
    }

    fn reload_canvas(&mut self) {
        self.load_canvas("");
    }

    /// Carga otro documento: el lienzo empieza arriba en vez de reponer el scroll anterior.
    fn reload_canvas_fresh(&mut self) {
        self.load_canvas("&fresh=1");
    }

    fn load_canvas(&mut self, extra: &str) {
        self.load_version += 1;
        if let Err(e) = self.canvas.load_url(&format!("{}{extra}", canvas_url(self.load_version))) {
            eprintln!("load_url falló: {e}");
        }
    }

    fn open_dialog(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        let mut dialog = FileDialog::new().add_filter("HTML", &["html", "htm"]);
        if let Some(dir) = self.path.as_deref().and_then(Path::parent) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.pick_file() else { return };
        match self.load_file(&path) {
            Ok(()) => self.status = t!("status.opened", path = path.display()),
            Err(e) => self.status = format!("⚠ {e}"),
        }
    }

    fn load_file(&mut self, path: &Path) -> Result<(), String> {
        let bytes = std::fs::read(path).map_err(|e| t!("status.read_failed", error = e))?;
        let text = String::from_utf8(bytes).map_err(|_| t("status.file_not_utf8"))?;
        self.install_document(Document::parse(text.trim_start_matches('\u{feff}')), Some(path.to_path_buf()));
        Ok(())
    }

    /// «Cerrar archivo»: pregunta por lo pendiente y deja un documento en blanco sin ruta.
    fn close_document(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        let name = self.file_name();
        self.install_document(Document::parse(&blank_html()), None);
        self.status = t!("status.closed", name = name);
    }

    /// Sustituye el documento actual por otro, con historial, selección y hojas abiertas a cero.
    fn install_document(&mut self, doc: Document, path: Option<PathBuf>) {
        self.doc = doc;
        self.doc_version += 1;
        self.history = History::default();
        self.undo_log.clear();
        self.redo_log.clear();
        self.selection = None;
        self.path = path;
        self.dirty = false;
        self.code_dirty = true;
        self.code_force = true;
        // Otro documento: sus hojas abiertas y su carpeta son otras.
        self.css_files.clear();
        self.active_file = None;
        self.sync_overrides();
        self.refresh_files();
        self.sync_shared();
        self.reload_canvas_fresh();
    }

    fn save(&mut self, as_new: bool) -> bool {
        let target = match (&self.path, as_new) {
            (Some(p), false) => p.clone(),
            _ => {
                let mut dialog = FileDialog::new()
                    .add_filter("HTML", &["html", "htm"])
                    .set_file_name(self.file_name());
                if let Some(dir) = self.path.as_deref().and_then(Path::parent) {
                    dialog = dialog.set_directory(dir);
                }
                match dialog.save_file() {
                    Some(p) => p,
                    None => return false,
                }
            }
        };
        // Temiktli reescribe el archivo normalizado: antes se conserva el original.
        let backup = match backup_original(&target) {
            Ok(b) => b,
            Err(e) => {
                self.status = t!("status.save_backup_failed", error = e);
                return false;
            }
        };
        let saved = self.doc.to_saved_html();
        match std::fs::write(&target, &saved.html) {
            Ok(()) => {
                let moved = self.path.as_deref().and_then(Path::parent) != target.parent();
                self.path = Some(target.clone());
                self.dirty = false;
                // Lo guardado es ahora el punto de partida de las próximas ediciones.
                self.doc.rebase_source(&saved.html);
                self.status = save_status(&target, backup.as_deref(), &saved.mode);
                let css_ok = self.save_css_files();
                if moved {
                    // Cambió la carpeta: los recursos relativos deben resolverse desde la nueva,
                    // y las hojas abiertas (ya guardadas donde estaban) dejan de ser de este sitio.
                    self.css_files.clear();
                    self.undo_log.retain(|s| matches!(s, Step::Doc));
                    self.redo_log.retain(|s| matches!(s, Step::Doc));
                    self.active_file = None;
                    self.code_dirty = true;
                    self.code_force = true;
                    self.sync_overrides();
                    self.sync_shared();
                    self.reload_canvas();
                }
                self.refresh_files();
                if self.auto_review {
                    self.review_after_save();
                }
                css_ok
            }
            Err(e) => {
                self.status = t!("status.save_failed", error = e);
                false
            }
        }
    }

    /// Escribe las hojas de estilo con cambios, cada una con su copia `.bak`. Añade el
    /// resultado a la barra de estado; `false` si alguna no se pudo guardar.
    fn save_css_files(&mut self) -> bool {
        let mut saved = 0;
        let mut errors = Vec::new();
        for css in self.css_files.iter_mut().filter(|c| c.dirty()) {
            let result = backup_original(&css.abs)
                .and_then(|_| std::fs::write(&css.abs, &css.text).map_err(|e| e.to_string()));
            match result {
                Ok(()) => {
                    css.saved = css.text.clone();
                    saved += 1;
                }
                Err(e) => errors.push(format!("{}: {e}", css.rel)),
            }
        }
        if saved > 0 {
            self.status.push_str(&t!("status.css_saved", n = saved));
        }
        if !errors.is_empty() {
            self.status.push_str(&t!("status.css_save_failed", list = errors.join("; ")));
        }
        errors.is_empty()
    }

    /// `true` si se puede continuar (nada pendiente, guardado, o descartado a propósito).
    fn confirm_discard(&mut self) -> bool {
        if !self.any_dirty() {
            return true;
        }
        let answer = MessageDialog::new()
            .set_level(MessageLevel::Warning)
            .set_title(t("dialog.unsaved_title"))
            .set_description(t!("dialog.unsaved_text", file = self.file_name()))
            .set_buttons(MessageButtons::YesNoCancel)
            .show();
        match answer {
            MessageDialogResult::Yes => self.save(false),
            MessageDialogResult::No => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn rel_within_gives_a_site_path_for_files_inside_only() {
        let base = std::env::temp_dir().join(format!("temiktli-relwithin-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let root = base.join("site");
        fs::create_dir_all(root.join("blog")).unwrap();
        fs::write(root.join("blog/p.html"), "").unwrap();
        fs::write(base.join("fuera.html"), "").unwrap();
        assert_eq!(rel_within(&root, &root.join("blog/p.html")).as_deref(), Some("blog/p.html"));
        // La raíz sin canonicalizar (como la guarda la aplicación) también vale.
        assert_eq!(rel_within(&root.join("blog").join(".."), &root.join("blog/p.html")).as_deref(), Some("blog/p.html"));
        assert_eq!(rel_within(&root, &base.join("fuera.html")), None);
        assert_eq!(rel_within(&root, &root.join("no-existe.html")), None);
    }

    #[test]
    fn open_files_follow_a_rename() {
        assert_eq!(rename_open_path("css/a.css", "css/a.css", "css/b.css", false).as_deref(), Some("css/b.css"));
        assert_eq!(rename_open_path("img/x/a.png", "img", "fotos", true).as_deref(), Some("fotos/x/a.png"));
        assert_eq!(rename_open_path("IMG/a.png", "img", "fotos", true).as_deref(), Some("fotos/a.png"));
        assert_eq!(rename_open_path("img2/a.png", "img", "fotos", true), None);
        assert_eq!(rename_open_path("css/a.css", "css/b.css", "css/c.css", false), None);
        // Un archivo no «contiene» a otros aunque comparta prefijo.
        assert_eq!(rename_open_path("css/a.css/x", "css/a.css", "css/b.css", false), None);
    }

    fn plan(from: &str, to: &str, is_dir: bool) -> fileops::RenamePlan {
        fileops::RenamePlan {
            from_rel: from.into(),
            to_rel: to.into(),
            from_abs: PathBuf::new(),
            to_abs: PathBuf::new(),
            is_dir,
            unchanged: false,
            ext_changed: false,
            exists: false,
            exists_is_dir: false,
        }
    }

    #[test]
    fn rename_summary_lists_affected_files_and_explains_the_buttons() {
        let im = fileops::Impact {
            referrers: vec![referrer("index.html", 2), referrer("blog/p.html", 1)],
            not_checked: 1,
            ..Default::default()
        };
        let text = rename_summary(&plan("css/main.css", "css/tema.css", false), &im);
        for part in ["“css/main.css”", "“css/tema.css”", "3 links", "2 files", "index.html (2)", "blog/p.html (1)",
            "Yes", "update", "No", "without touching", "Cancel", "1 file could not be checked"]
        {
            assert!(text.contains(part), "falta «{part}» en: {text}");
        }
        let text = rename_summary(&plan("img", "fotos", true), &im);
        assert!(text.contains("folder “img”"), "{text}");
    }

    #[test]
    fn replace_text_warns_about_unsaved_sheets() {
        let mut p = plan("css/a.css", "css/b.css", false);
        p.exists = true;
        let text = replace_text(&p, 0);
        assert!(text.contains("The file “css/b.css”") && text.contains("Recycle Bin"), "{text}");
        assert!(!text.contains("unsaved"), "{text}");
        p.exists_is_dir = true;
        let text = replace_text(&p, 2);
        assert!(text.contains("The folder “css/b.css”"), "{text}");
        assert!(text.contains("2 open stylesheets") && text.contains("unsaved") && text.contains("lost"), "{text}");
        assert!(replace_text(&p, 1).contains("1 open stylesheet with"), "{}", replace_text(&p, 1));
    }

    #[test]
    fn unchecked_text_says_how_many_files_were_not_checked() {
        let p = plan("css/a.css", "css/b.css", false);
        let one = unchecked_text(&p, 1);
        assert!(one.contains("“css/a.css”") && one.contains("1 file could not be checked"), "{one}");
        assert!(unchecked_text(&p, 3).contains("3 files could not be checked"));
    }

    #[test]
    fn undo_steps_follow_a_renamed_sheet() {
        let css = |rel: &str| Step::Css(CssEdit { rel: rel.into(), before: "a".into(), after: "b".into() });
        let rel_of = |s: &Step| match s {
            Step::Css(e) => e.rel.clone(),
            Step::Doc => "doc".into(),
        };
        let mut steps = vec![css("css/main.css"), Step::Doc, css("img2/x.css"), css("css/sub/b.css")];
        remap_steps(&mut steps, "css", "estilos", true);
        let rels: Vec<_> = steps.iter().map(rel_of).collect();
        assert_eq!(rels, ["estilos/main.css", "doc", "img2/x.css", "estilos/sub/b.css"]);
        let mut one = vec![css("a.css")];
        remap_steps(&mut one, "a.css", "b.css", false);
        assert_eq!(rel_of(&one[0]), "b.css");
    }

    fn referrer(rel: &str, links: usize) -> fileops::Referrer {
        fileops::Referrer { rel: rel.into(), links }
    }

    #[test]
    fn delete_summary_for_a_loose_file() {
        let text = delete_summary("css/main.css", false, &fileops::Impact::default(), 0);
        assert!(text.contains("“css/main.css”") && text.contains("Recycle Bin"), "{text}");
        assert!(!text.contains("link") && !text.contains("unsaved"), "{text}");
    }

    #[test]
    fn delete_summary_for_a_folder_lists_contents_and_broken_links() {
        let im = fileops::Impact {
            referrers: vec![referrer("a.html", 2), referrer("blog/b.html", 1)],
            not_checked: 0,
            files: 3,
            dirs: 1,
        };
        let text = delete_summary("img", true, &im, 0);
        for part in ["folder “img”", "3 files", "1 subfolder", "3 links", "2 files", "a.html (2)", "blog/b.html (1)"] {
            assert!(text.contains(part), "falta «{part}» en: {text}");
        }
        let one = fileops::Impact { files: 1, dirs: 2, ..Default::default() };
        let text = delete_summary("img", true, &one, 0);
        assert!(text.contains("1 file") && text.contains("2 subfolders"), "{text}");
    }

    #[test]
    fn delete_summary_warns_about_unchecked_files_and_unsaved_sheets() {
        let im = fileops::Impact { not_checked: 2, ..Default::default() };
        let text = delete_summary("a.png", false, &im, 0);
        assert!(text.contains("2 files") && text.contains("could not be checked"), "{text}");
        let text = delete_summary("css", true, &fileops::Impact::default(), 1);
        assert!(text.contains("1 open stylesheet") && text.contains("unsaved"), "{text}");
    }

    #[test]
    fn delete_summary_cuts_a_long_list_of_referrers() {
        let many: Vec<_> = (0..12).map(|i| referrer(&format!("p{i}.html"), 1)).collect();
        let im = fileops::Impact { referrers: many, ..Default::default() };
        let text = delete_summary("x.png", false, &im, 0);
        assert!(text.contains("p0.html") && !text.contains("p11.html") && text.contains("and 4 more"), "{text}");
    }

    #[test]
    fn code_page_embeds_the_highlighter_without_breaking_out_of_its_script() {
        let page = code_page();
        for marker in ["/*HIGHLIGHT_JS*/", "/*COMPLETE_JS*/", "/*FIND_JS*/"] {
            assert!(!page.contains(marker), "{marker} sin sustituir");
        }
        assert!(page.contains("root.highlightHtml") && page.contains("root.htmlComplete"));
        assert!(page.contains("root.htmlFind"));
        // Cinco bloques <script> (textos, coloreador, autocompletado, búsqueda y lógica):
        // cualquier otro cierre dentro del JS insertado rompería la página.
        assert_eq!(page.matches("</script").count(), 5);
    }

    #[test]
    fn the_properties_breadcrumb_selects_through_the_same_message_as_the_canvas() {
        assert!(PANEL_HTML_SRC.contains("sel.crumbs") && PANEL_HTML_SRC.contains("className: 'crumb"));
        assert!(PANEL_HTML_SRC.contains("type: 'Select', id: c.id"));
        let m: FromWebview = serde_json::from_str(r#"{"type":"Select","id":7}"#).unwrap();
        assert!(matches!(m, FromWebview::Select { id: 7 }));
    }

    #[test]
    fn dragging_the_blue_tab_talks_to_rust_with_messages_it_understands() {
        let js = include_str!("../assets/editor-inject.js");
        for ty in ["DragOver", "MoveElement", "DragEnd"] {
            assert!(js.contains(&format!("type: '{ty}'")), "{ty} falta en el lienzo");
        }
        assert!(js.contains("DropState(p)") && js.contains("startDrag"));
        for json in [
            r#"{"type":"DragOver","id":4,"target":5,"place":"after"}"#,
            r#"{"type":"MoveElement","id":4,"target":5,"place":"inside"}"#,
            r#"{"type":"DragEnd"}"#,
        ] {
            assert!(serde_json::from_str::<FromWebview>(json).is_ok(), "{json}");
        }
        let ok = serde_json::to_string(&Patch::DropState { ok: false, why: Some("no".into()) }).unwrap();
        assert_eq!(ok, r#"{"type":"DropState","ok":false,"why":"no"}"#);
    }

    #[test]
    fn a_document_without_a_file_is_called_new_html_in_each_language() {
        assert_eq!(t("app.new_file_name"), "new.html");
        let es = i18n::Dict::parse(i18n::LANGUAGES.iter().find(|(c, _)| *c == "es").unwrap().1).unwrap();
        assert_eq!(es.get("app.new_file_name"), Some("nuevo.html"));
        assert!(!include_str!("main.rs").contains(concat!("Sin ", "título.html")));
    }

    #[test]
    fn the_language_setting_travels_in_the_state_and_the_menu_sends_it_back() {
        for value in ["system", "es", "pl"] {
            let json = format!(r#"{{"type":"SetLanguage","lang":"{value}"}}"#);
            assert!(matches!(serde_json::from_str::<FromWebview>(&json), Ok(FromWebview::SetLanguage { lang }) if lang == value));
        }
        assert!(POPUP_HTML_SRC.contains("type: 'SetLanguage'") && POPUP_HTML_SRC.contains("data-sub=\"language\""));
        assert!(POPUP_HTML_SRC.contains("state.language"));
        // Cada página lleva su diccionario incrustado: ni marcador ni claves sin resolver.
        for page in [code_page(), panel_page(), popup_page(), toolbar_page(), inject_js()] {
            assert!(!page.contains("/*I18N_JS*/") && !page.contains("/*I18N_JSON*/"), "marcador sin sustituir");
            assert!(page.contains("\"menu.file.open\":\"Open…\"") || page.contains("\"canvas.no_box\""), "sin diccionario");
        }
    }

    #[test]
    fn the_new_document_has_the_requested_structure() {
        assert_eq!(
            blank_html(),
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title></title>\n</head>\n<body>\n</body>\n</html>\n"
        );
        let doc = Document::parse(&blank_html());
        assert!(doc.find_tag("head").is_some() && doc.find_tag("body").is_some());
    }

    #[test]
    fn the_insert_mark_follows_the_same_rule_as_inserting() {
        let doc = Document::parse("<html><head></head><body><div id=\"a\"><p>x</p></div><p id=\"b\">y</p></body></html>");
        let body = doc.find_tag("body").unwrap();
        // Sin selección: al final del cuerpo.
        assert_eq!(insert_mark(&doc, None), Patch::InsertMark { parent: Some(body), index: 2 });
        // Un párrafo (solo texto) no admite hijos: va justo después.
        let b = doc.child_at(body, 1).unwrap();
        assert_eq!(insert_mark(&doc, Some(b)), Patch::InsertMark { parent: Some(body), index: 2 });
        // Un div admite hijos: va dentro, al final.
        let a = doc.child_at(body, 0).unwrap();
        assert_eq!(insert_mark(&doc, Some(a)), Patch::InsertMark { parent: Some(a), index: 1 });
        // Y es exactamente lo que hace insertion_point.
        assert_eq!(doc.insertion_point("div", Some(a)), Ok((a, 1)));
    }

    #[test]
    fn the_canvas_script_draws_the_insert_mark() {
        let js = include_str!("../assets/editor-inject.js");
        assert!(js.contains("InsertMark(p)") && js.contains("data-tk-insertmark"));
        let json = serde_json::to_string(&Patch::InsertMark { parent: Some(3), index: 1 }).unwrap();
        assert_eq!(json, r#"{"type":"InsertMark","parent":3,"index":1}"#);
    }

    #[test]
    fn the_window_icon_data_is_a_valid_rgba_image() {
        assert_eq!(include_bytes!("../assets/icon-64.rgba").len() as u32, ICON_SIDE * ICON_SIDE * 4);
        assert!(window_icon().is_some());
    }

    #[test]
    fn packaging_icons_exist_and_are_valid() {
        let icns: &[u8] = include_bytes!("../assets/icon.icns");
        assert_eq!(&icns[..4], b"icns");
        let png: &[u8] = include_bytes!("../assets/icon-512.png");
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        assert!(width >= 512, "icon-512.png mide {width} px de ancho");
    }

    #[test]
    fn startup_failure_text_names_the_error_and_the_runtime_to_install() {
        let text = startup_failure_text("COM error 0x80070002");
        assert!(text.contains("COM error 0x80070002"), "{text}");
        assert!(text.contains("developer.microsoft.com/microsoft-edge/webview2"), "{text}");
        assert!(text.contains("libwebkit2gtk-4.1"), "{text}");
    }

    #[test]
    fn the_initial_window_keeps_room_below_it() {
        let area = (0, 0, 1920, 1040); // pantalla de 1080 menos una barra de tareas de 40
        // Cabe: solo se centra.
        assert_eq!(fit_rect((1280, 800), area, 20), (320, 110, 1280, 800));
        // Demasiado alta: se encoge para dejar 20 libres abajo.
        let (_, y, _, h) = fit_rect((1280, 1100), area, 20);
        assert_eq!((y, h), (0, 1020));
        assert!(y + h as i32 <= area.3 - 20);
        // Demasiado ancha: no se sale por los lados.
        let (x, _, w, _) = fit_rect((3000, 800), area, 20);
        assert_eq!((x, w), (0, 1920));
        // Un área con origen distinto (segundo monitor a la izquierda) se respeta.
        assert_eq!(fit_rect((1000, 700), (-1920, 0, 0, 1000), 20).0, -1460);
    }

    #[test]
    fn arrangements_split_the_window_without_overlap() {
        let (w, h, scale) = (1600, 1000, 1.0);
        let panel_w = PANEL_WIDTH_LOGICAL as u32;
        let area = w - panel_w;
        let gap = divider_px(scale);
        let bar = toolbar_px(scale);
        let rest = h - bar; // lo que queda bajo la barra de herramientas
        let f = Fractions::default();

        let z = boxes((w, h), scale, None, Arrangement::Below, f);
        // La barra ocupa todo el ancho, arriba; todo lo demás cuelga de ella.
        assert_eq!(z.toolbar, (0, 0, w, bar));
        assert_eq!(z.panel, (area as i32, bar as i32, panel_w, rest));
        // Lienzo, franja y código se suceden de arriba abajo, sin huecos ni solapes.
        assert_eq!(z.canvas.1, bar as i32);
        assert_eq!(z.divider, Some((0, bar as i32 + z.canvas.3 as i32, area, gap)));
        assert_eq!((z.code.0, z.code.1 as u32, z.code.2), (0, bar + z.canvas.3 + gap, area));
        assert_eq!(z.canvas.3 + gap + z.code.3, rest);

        let z = boxes((w, h), scale, None, Arrangement::Side, f);
        assert_eq!(z.divider, Some((z.canvas.2 as i32, bar as i32, gap, rest)));
        assert_eq!(z.code.0 as u32, z.canvas.2 + gap);
        assert_eq!(z.canvas.2 + gap + z.code.2, area);
        assert_eq!((z.canvas.3, z.code.3), (rest, rest));

        let z = boxes((w, h), scale, None, Arrangement::CodeOnly, f);
        assert_eq!((z.canvas.2, z.canvas.3, z.divider), (0, 0, None));
        assert_eq!((z.code.2, z.code.3), (area, rest));

        let z = boxes((w, h), scale, None, Arrangement::CanvasOnly, f);
        assert_eq!((z.code.2, z.code.3, z.divider), (0, 0, None));
        assert_eq!((z.canvas.2, z.canvas.3), (area, rest));
    }

    #[test]
    fn a_menu_opens_under_its_button_and_never_leaves_the_window() {
        let (size, scale) = ((1600, 1000), 1.25);
        let bar = toolbar_px(scale) as i32;
        // Bajo la barra, con su borde izquierdo donde el botón.
        let p = popup_px(size, scale, "file", 200.0, None);
        assert_eq!((p.0, p.1), ((200.0 * scale) as i32, bar));
        // Un botón junto al borde derecho: el menú se desplaza para caber.
        let p = popup_px(size, scale, "search", 1500.0, None);
        assert_eq!(p.0 as u32 + p.2, size.0);
        // En una ventana pequeña, el menú se encoge hasta lo que hay.
        let p = popup_px((300, 200), scale, "review", 10.0, None);
        assert!(p.2 <= 300 && p.3 as i32 + p.1 <= 200, "{p:?}");
        // Un nombre desconocido no rompe nada.
        let _ = popup_px(size, scale, "nada", 0.0, None);
        // Si el menú dice cuánto necesita (p. ej. con un submenú abierto), se respeta.
        let p = popup_px(size, scale, "view", 100.0, Some((420.0, 150.0)));
        assert_eq!((p.2, p.3), ((420.0 * scale) as u32, (150.0 * scale) as u32));
    }

    #[test]
    fn a_narrow_canvas_is_centered_in_the_space_it_gets() {
        let f = Fractions::default();
        let gap = divider_px(1.0);
        let z = boxes((1600, 1000), 1.0, Some(375), Arrangement::Side, f);
        assert_eq!(z.canvas.2, 375);
        let left_w = z.code.0 as u32 - gap;
        assert_eq!(z.canvas.0 as u32, (left_w - 375) / 2);
        // Más ancho que el espacio disponible: se recorta, sin salirse.
        let z = boxes((1600, 1000), 1.0, Some(1024), Arrangement::Side, f);
        assert_eq!((z.canvas.0, z.canvas.2), (0, z.code.0 as u32 - gap));
    }

    #[test]
    fn dragging_the_divider_gives_the_code_view_the_space_under_the_pointer() {
        let (w, h, scale) = (1600, 1000, 1.0);
        let gap = f64::from(divider_px(scale));
        // Abajo: el centro de la franja sigue al ratón.
        let f = fraction_at((w, h), scale, Arrangement::Below, (300.0, 400.0)).unwrap();
        let z = boxes((w, h), scale, None, Arrangement::Below, Fractions { below: f, side: 0.45 });
        let centre = f64::from(z.divider.unwrap().1) + gap / 2.0;
        assert!((centre - 400.0).abs() <= 1.0, "centro {centre}");
        // Al lado, lo mismo en horizontal.
        let f = fraction_at((w, h), scale, Arrangement::Side, (500.0, 300.0)).unwrap();
        let z = boxes((w, h), scale, None, Arrangement::Side, Fractions { below: 0.38, side: f });
        let centre = f64::from(z.divider.unwrap().0) + gap / 2.0;
        assert!((centre - 500.0).abs() <= 1.0, "centro {centre}");
        // Ni el lienzo ni el código desaparecen por arrastrar hasta el borde.
        assert_eq!(fraction_at((w, h), scale, Arrangement::Below, (0.0, 0.0)), Some(MAX_FRACTION));
        assert_eq!(fraction_at((w, h), scale, Arrangement::Below, (0.0, 999.0)), Some(MIN_FRACTION));
        // Sin divisor no hay nada que arrastrar.
        assert_eq!(fraction_at((w, h), scale, Arrangement::CodeOnly, (1.0, 1.0)), None);
        assert_eq!(fraction_at((w, h), scale, Arrangement::CanvasOnly, (1.0, 1.0)), None);
    }

    #[test]
    fn the_divider_is_only_where_the_window_is_not_covered_by_a_view() {
        let z = boxes((1600, 1000), 1.25, None, Arrangement::Below, Fractions::default());
        let d = z.divider.unwrap();
        let mid = (f64::from(d.0) + 10.0, f64::from(d.1) + f64::from(d.3) / 2.0);
        assert!(inside(d, mid));
        // Un punto del lienzo o del código no es el divisor.
        assert!(!inside(d, (mid.0, f64::from(z.canvas.1) + 10.0)));
        assert!(!inside(d, (mid.0, f64::from(z.code.1) + 10.0)));
        // Y con tamaños diminutos nada se sale de la ventana ni da un desbordamiento.
        for (w, h) in [(0, 0), (10, 10), (400, 30), (30, 400)] {
            for a in [Arrangement::Below, Arrangement::Side] {
                let _ = boxes((w, h), 1.25, Some(375), a, Fractions::default());
            }
        }
    }

    #[test]
    fn arrangement_names_round_trip_and_unknown_ones_are_rejected() {
        for a in [Arrangement::Below, Arrangement::Side, Arrangement::CodeOnly, Arrangement::CanvasOnly] {
            assert_eq!(Arrangement::parse(a.name()), Some(a));
        }
        assert_eq!(Arrangement::parse("diagonal"), None);
        assert!(Arrangement::Below.code_visible() && !Arrangement::CanvasOnly.code_visible());
        assert!(Arrangement::Side.canvas_visible() && !Arrangement::CodeOnly.canvas_visible());
    }

    #[test]
    fn utf16_positions_round_trip_with_multibyte_text() {
        let t = "ñ<a>\n😀<b>";
        let b = t.find("<b>").unwrap();
        assert_eq!(line_col_utf16(t, b), (1, 2)); // 😀 ocupa 2 unidades UTF-16
        assert_eq!(line_col_utf16(t, t.find("<a>").unwrap()), (0, 1));
        assert_eq!(utf16_to_byte(t, 1), "ñ".len());
        assert_eq!(utf16_to_byte(t, 5 + 2), b); // «ñ<a>\n» son 5 unidades, más 😀
        assert_eq!(utf16_to_byte(t, 999), t.len());
    }

    #[test]
    fn popup_page_embeds_the_search_engine_without_breaking_out_of_its_script() {
        let page = popup_page();
        assert!(!page.contains("/*FIND_JS*/"), "marcador sin sustituir");
        assert!(page.contains("root.htmlFind"));
        // Tres bloques <script> (textos, motor de búsqueda y lógica de los menús).
        assert_eq!(page.matches("</script").count(), 3);
        // El panel ya no lleva el motor: no tiene búsqueda.
        assert!(!panel_page().contains("htmlFind"));
    }

    /// Regresión de un bloqueo real: cada `MenuSize` que manda el menú provoca una actualización de
    /// estado hacia él; si esa actualización lo redibuja y vuelve a avisar, el bucle no termina nunca
    /// y la aplicación deja de responder. Se comprobó simulándolo en un navegador; aquí se vigila
    /// que las dos cortafuegos del código siguen en su sitio.
    #[test]
    fn the_menu_never_answers_a_state_update_with_a_size_report_that_changes_nothing() {
        // 1) Solo avisa del tamaño cuando cambia de verdad…
        assert!(POPUP_HTML_SRC.contains("if (key === lastReported) return;"));
        // 2) …y reinicia ese recuerdo al abrir un menú (Rust olvida el tamaño al abrirlo).
        assert!(POPUP_HTML_SRC.contains("lastReported = '';"));
        // 3) Y la actualización de estado no redibuja los snippets si nada cambió para ellos.
        assert!(POPUP_HTML_SRC.contains("const changed = canSave !== lastCanSave;"));
        // Dentro de `update` no hay ninguna llamada directa a redibujar sin condición.
        let update = POPUP_HTML_SRC.split("update(s) {").nth(1).expect("update");
        let update = update.split("snippets(list) {").next().unwrap();
        assert!(update.matches("renderSnippets()").count() == 1 && update.contains("changed &&"));
    }

    #[test]
    fn the_snippet_menu_gets_names_and_which_ones_are_built_in_but_never_the_html() {
        let all = vec![
            snippets::Snippet { name: "De serie".into(), html: "<p>secreto?</p>".into() },
            snippets::Snippet { name: "Mío".into(), html: "<hr>".into() },
        ];
        let menu = snippet_menu(&all, 1);
        assert_eq!(menu[0], serde_json::json!({ "name": "De serie", "builtin": true }));
        assert_eq!(menu[1], serde_json::json!({ "name": "Mío", "builtin": false }));
        assert!(!serde_json::to_string(&menu).unwrap().contains("<p>"));
        // La página usa justo esos nombres de campo.
        assert!(POPUP_HTML_SRC.contains("s.builtin") && POPUP_HTML_SRC.contains("s.name"));
        for msg in ["SaveSnippet", "InsertSnippet", "DeleteSnippet"] {
            assert!(POPUP_HTML_SRC.contains(&format!("type: '{msg}'")), "{msg}");
        }
        // Y los de serie son los que Rust cuenta como tales.
        assert_eq!(snippet_menu(&snippets::builtins(), snippets::builtins().len()).iter().filter(|m| m["builtin"] == true).count(), 5);
    }

    #[test]
    fn the_review_menu_has_the_html_cleanup_button_wired_to_rust() {
        assert!(POPUP_HTML_SRC.contains("id=\"cleanhtml\"") && POPUP_HTML_SRC.contains("type: 'CleanHtml'"));
        let m: FromWebview = serde_json::from_str(r#"{"type":"CleanHtml"}"#).unwrap();
        assert!(matches!(m, FromWebview::CleanHtml));
    }

    #[test]
    fn the_review_menu_has_the_indent_and_minify_buttons_wired_to_rust() {
        for (id, msg) in [("indenthtml", "IndentHtml"), ("minifyhtml", "MinifyHtml")] {
            assert!(POPUP_HTML_SRC.contains(&format!("id=\"{id}\"")), "{id}");
            assert!(POPUP_HTML_SRC.contains(&format!("type: '{msg}'")), "{msg}");
        }
        // Van en el menú Revisión, debajo de «Limpiar HTML (Word)».
        let at = |s: &str| POPUP_HTML_SRC.find(s).unwrap_or_else(|| panic!("falta {s}"));
        assert!(at("id=\"cleanhtml\"") < at("id=\"indenthtml\"") && at("id=\"indenthtml\"") < at("id=\"minifyhtml\""));
        assert!(at("id=\"minifyhtml\"") < at("id=\"rsummary\""));
    }

    #[test]
    fn size_summary_shows_the_saving() {
        // En inglés el decimal es un punto; en español, `$meta.decimal_separator` lo vuelve coma.
        assert_eq!(size_summary(2048, 1024), "2.0 KB → 1.0 KB (-50 %)");
        assert_eq!(size_summary(900, 600), "900 B → 600 B (-33 %)");
        assert_eq!(size_summary(1000, 1000), "1000 B → 1000 B (no change)");
        assert_eq!(size_summary(0, 0), "0 B → 0 B (no change)");
        assert!(size_summary(100, 150).contains("+50 %"));
    }

    #[test]
    fn auto_review_has_its_switch_and_its_indicator() {
        // La casilla del menú Revisión y el indicador del botón de la barra existen.
        assert!(POPUP_HTML_SRC.contains("id=\"autorev\""));
        assert!(POPUP_HTML_SRC.contains("type: 'SetAutoReview'"));
        assert!(TOOLBAR_HTML.contains("id=\"badge\"") && TOOLBAR_HTML.contains("s.review"));
        // El estado que reciben las vistas lleva el resultado de la revisión.
        let counts = ReviewCounts { errors: 2, warnings: 0, infos: 1 };
        let json = serde_json::to_string(&counts).unwrap();
        assert_eq!(json, r#"{"errors":2,"warnings":0,"infos":1}"#);
        // Los nombres que lee la barra son los que Rust envía.
        for field in ["errors", "warnings", "infos"] {
            assert!(TOOLBAR_HTML.contains(&format!("r.{field}")), "{field}");
        }
    }

    #[test]
    fn the_tag_menu_is_built_from_the_catalog_the_editor_can_create() {
        let page = popup_page();
        assert!(!page.contains("/*TAGS_JSON*/"), "marcador sin sustituir");
        // El JSON insertado es una lista de { name, tags, groups } con los nombres traducidos.
        let json = temiktli_core::tag_menu_json().to_string();
        assert!(page.contains(&format!("const CATEGORIES = {json};")));
        assert_eq!(temiktli_core::TAG_CATEGORIES.len(), 16);
        // Cada etiqueta del menú, esté suelta o en un subgrupo, es una que el editor sabe crear
        // (y «canva» era canvas).
        let all: Vec<&str> = temiktli_core::TAG_CATEGORIES.iter().flat_map(|c| c.all_tags()).collect();
        assert!(all.iter().all(|t| temiktli_core::is_insertable(t)));
        assert!(all.contains(&"canvas") && !all.contains(&"canva"));
        // Formato va repartido en subgrupos y llega así al menú.
        assert!(page.contains("\"groups\":[[\"Emphasis and style\""));
        // Sigue habiendo tres bloques <script> en la página, así que el JSON no rompe nada.
        assert_eq!(page.matches("</script").count(), 3);
    }

    #[test]
    fn every_menu_the_toolbar_opens_exists_in_the_popup() {
        for name in ["file", "edit", "search", "tags", "snippets", "view", "review"] {
            assert!(TOOLBAR_HTML.contains(&format!("data-menu=\"{name}\"")), "{name} falta en la barra");
            assert!(POPUP_HTML_SRC.contains(&format!("id=\"m-{name}\"")), "{name} falta en el menú");
        }
        // Insertar etiquetas va justo después de Buscar archivos.
        let pos = |m: &str| TOOLBAR_HTML.find(&format!("data-menu=\"{m}\"")).unwrap();
        assert!(pos("search") < pos("tags") && pos("tags") < pos("view"));
        // Snippets va a continuación de Insertar etiquetas.
        assert!(pos("tags") < pos("snippets") && pos("snippets") < pos("view"));
        // Insertar volvió al panel: ya no es un menú.
        assert!(!TOOLBAR_HTML.contains("data-menu=\"insert\""));
        assert!(PANEL_HTML_SRC.contains("id=\"insert\""));
        // Cada orden de Archivo y Edición es un mensaje que Rust entiende.
        for msg in ["Open", "Save", "SaveAs", "CloseDocument", "Copy", "Paste", "Undo", "Redo"] {
            assert!(POPUP_HTML_SRC.contains(&format!("data-msg=\"{msg}\"")), "{msg} falta en el menú");
            let json = format!(r#"{{"type":"{msg}"}}"#);
            assert!(serde_json::from_str::<FromWebview>(&json).is_ok(), "{msg} no se entiende");
        }
        // Los submenús de Vista ofrecen todo lo que antes había en las listas.
        for value in ["375", "768", "1024", "below", "side", "code", "canvas", "system", "light", "dark"] {
            assert!(POPUP_HTML_SRC.contains(&format!("data-value=\"{value}\"")), "{value} falta en Vista");
        }
    }

    #[test]
    fn save_status_warns_only_when_the_file_was_reformatted() {
        let t = Path::new("a.html");
        let ok = save_status(t, Some(Path::new("a.html.bak")), &SaveMode::Preserved);
        assert!(ok.contains("original formatting kept") && ok.contains("a.html.bak"));
        assert!(!ok.contains('⚠'));
        let bad = save_status(t, None, &SaveMode::Normalized("motivo".into()));
        assert!(bad.contains('⚠') && bad.contains("motivo"));
    }

    /// Carpeta temporal única con `site/index.css` y un `secreto.txt` fuera de `site`.
    fn fixture(name: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("temiktli-test-{name}-{}", std::process::id()));
        let site = base.join("site");
        fs::create_dir_all(&site).unwrap();
        fs::write(site.join("index.css"), "body{}").unwrap();
        fs::write(base.join("secreto.txt"), "no debe salir").unwrap();
        (base, site)
    }

    #[test]
    fn serves_files_inside_root() {
        let (base, site) = fixture("ok");
        let (bytes, mime) = read_site_file(Some(&site), "/index.css").unwrap();
        assert_eq!(bytes, b"body{}");
        assert!(mime.starts_with("text/css"));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn refuses_path_traversal() {
        let (base, site) = fixture("trav");
        for evil in ["/../secreto.txt", "/..%2Fsecreto.txt", "/..%5Csecreto.txt", "/sub/../../secreto.txt"] {
            assert!(read_site_file(Some(&site), &percent_decode(evil)).is_none(), "{evil}");
        }
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn refuses_without_root_or_for_missing_files() {
        let (base, site) = fixture("none");
        assert!(read_site_file(None, "/index.css").is_none());
        assert!(read_site_file(Some(&site), "/nope.css").is_none());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn backup_copies_original_bytes_once_and_never_overwrites() {
        let (base, site) = fixture("bak");
        let file = site.join("index.html");
        let original = "<HTML lang=es>\r\n<p>sin cerrar".as_bytes(); // CRLF y mayúsculas intactos
        fs::write(&file, original).unwrap();

        let bak = backup_original(&file).unwrap().expect("primera vez: crea la copia");
        assert_eq!(bak.file_name().unwrap(), "index.html.bak");
        assert_eq!(fs::read(&bak).unwrap(), original);

        // Tras guardar (el archivo cambia), una segunda copia NO debe pisar el original.
        fs::write(&file, "<html>normalizado</html>").unwrap();
        assert_eq!(backup_original(&file).unwrap(), None);
        assert_eq!(fs::read(&bak).unwrap(), original);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn backup_is_skipped_for_new_files_and_directories() {
        let (base, site) = fixture("bak2");
        assert_eq!(backup_original(&site.join("nuevo.html")).unwrap(), None);

        // Un directorio no es un archivo: no se finge una copia.
        assert_eq!(backup_original(&site).unwrap(), None);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn percent_decode_handles_edge_cases() {
        assert_eq!(percent_decode("/a%20b"), "/a b");
        assert_eq!(percent_decode("/100%"), "/100%");
        assert_eq!(percent_decode("/%zz"), "/%zz");
        assert_eq!(percent_decode("/%é"), "/%é"); // no debe entrar en pánico
    }
}

// ---------- Ventana ----------

/// El destino del próximo elemento para el lienzo. Se calcula como un bloque genérico (`div`):
/// es lo que ocurre con casi todas las etiquetas; las que tienen reglas propias (`li`, `td`,
/// `title`…) se colocan al insertarlas y el mensaje de estado lo dice.
fn insert_mark(doc: &Document, selection: Option<NodeId>) -> Patch {
    match doc.insertion_point("div", selection) {
        Ok((parent, index)) => Patch::InsertMark { parent: Some(parent), index },
        Err(_) => Patch::InsertMark { parent: None, index: 0 },
    }
}

/// Lado (px) de `assets/icon-64.rgba`: píxeles RGBA crudos, sin cabecera.
const ICON_SIDE: u32 = 64;

/// Icono de la ventana (barra de título, barra de tareas, Alt+Tab).
fn window_icon() -> Option<tao::window::Icon> {
    tao::window::Icon::from_rgba(include_bytes!("../assets/icon-64.rgba").to_vec(), ICON_SIDE, ICON_SIDE).ok()
}

/// Espacio mínimo (lógico) que la ventana inicial deja libre bajo ella, sobre la barra de tareas.
const BOTTOM_MARGIN_LOGICAL: f64 = 20.0;

/// Área útil del monitor principal (sin la barra de tareas) como `(izq, arr, der, abajo)` físicos.
#[cfg(windows)]
fn work_area() -> Option<(i32, i32, i32, i32)> {
    #[repr(C)]
    struct Rect32 {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SystemParametersInfoW(action: u32, param: u32, data: *mut Rect32, win_ini: u32) -> i32;
    }
    const SPI_GETWORKAREA: u32 = 0x0030;
    let mut r = Rect32 { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: `r` es un RECT válido y vivo durante la llamada, que solo escribe en él.
    let ok = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut r, 0) };
    (ok != 0 && r.right > r.left && r.bottom > r.top).then_some((r.left, r.top, r.right, r.bottom))
}

#[cfg(not(windows))]
fn work_area() -> Option<(i32, i32, i32, i32)> {
    None
}

/// Tamaño y posición exteriores `(x, y, ancho, alto)` para una ventana de `outer` (ancho, alto)
/// dentro de `area` (izq, arr, der, abajo): se encoge lo justo para dejar `margin` libre abajo y
/// se centra en lo que queda.
fn fit_rect(outer: (u32, u32), area: (i32, i32, i32, i32), margin: i32) -> (i32, i32, u32, u32) {
    let avail_w = (area.2 - area.0).max(1) as u32;
    let avail_h = (area.3 - area.1 - margin).max(1) as u32;
    let (w, h) = (outer.0.min(avail_w), outer.1.min(avail_h));
    let x = area.0 + ((avail_w - w) / 2) as i32;
    let y = area.1 + ((avail_h - h) / 2) as i32;
    (x, y, w, h)
}

/// Acomoda la ventana recién creada para que no se salga del monitor ni tape la barra de tareas.
fn fit_window_to_screen(window: &Window) {
    let area = work_area().or_else(|| {
        let m = window.primary_monitor()?;
        let (p, s) = (m.position(), m.size());
        Some((p.x, p.y, p.x + s.width as i32, p.y + s.height as i32))
    });
    let Some(area) = area else { return };
    let margin = (BOTTOM_MARGIN_LOGICAL * window.scale_factor()).round() as i32;
    let outer = window.outer_size();
    let (x, y, w, h) = fit_rect((outer.width, outer.height), area, margin);
    if (w, h) != (outer.width, outer.height) {
        // El marco y la barra de título no cuentan en el tamaño interior: se resta la diferencia.
        let inner = window.inner_size();
        window.set_inner_size(PhysicalSize::new(
            inner.width.saturating_sub(outer.width - w).max(1),
            inner.height.saturating_sub(outer.height - h).max(1),
        ));
    }
    window.set_outer_position(PhysicalPosition::new(x, y));
}

fn rect(x: i32, y: i32, w: u32, h: u32) -> Rect {
    Rect {
        position: Position::Physical(PhysicalPosition::new(x, y)),
        size: Size::Physical(PhysicalSize::new(w, h)),
    }
}

/// Pinta de negro el fondo de la ventana que no cubre ningún webview. Al cambiar el tamaño o
/// la posición de un webview, Windows deja ahí lo que había antes (p. ej. una franja blanca
/// del lienzo cuando se estrecha). Los webviews son ventanas hijas y quedan excluidos del
/// pintado, así que solo se toca el hueco.
#[cfg(windows)]
fn paint_window_background(window: &Window) {
    use tao::platform::windows::WindowExtWindows;
    use std::ffi::c_void;
    #[repr(C)]
    struct Rc {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetDC(hwnd: *mut c_void) -> *mut c_void;
        fn ReleaseDC(hwnd: *mut c_void, hdc: *mut c_void) -> i32;
        fn GetClientRect(hwnd: *mut c_void, rect: *mut Rc) -> i32;
        fn FillRect(hdc: *mut c_void, rect: *const Rc, brush: *mut c_void) -> i32;
    }
    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn CreateSolidBrush(color: u32) -> *mut c_void;
        fn DeleteObject(object: *mut c_void) -> i32;
    }
    // COLORREF es 0x00BBGGRR: #2b2f36, un gris oscuro que se lee como divisor entre vistas.
    const BACKGROUND: u32 = 0x0036_2f2b;
    let hwnd = window.hwnd() as isize as *mut c_void;
    let mut r = Rc { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: `hwnd` es la ventana viva de `window`; el DC y el pincel creados aquí se
    // liberan antes de salir.
    unsafe {
        if GetClientRect(hwnd, &mut r) == 0 {
            return;
        }
        let hdc = GetDC(hwnd);
        if hdc.is_null() {
            return;
        }
        let brush = CreateSolidBrush(BACKGROUND);
        FillRect(hdc, &r, brush);
        DeleteObject(brush);
        ReleaseDC(hwnd, hdc);
    }
}

#[cfg(not(windows))]
fn paint_window_background(_window: &Window) {}

/// Mientras se arrastra el divisor, el ratón se «captura» para seguir recibiendo sus
/// movimientos aunque pase por encima de un webview (que de otro modo se los quedaría).
#[cfg(windows)]
fn capture_mouse(window: &Window, on: bool) {
    use std::ffi::c_void;
    use tao::platform::windows::WindowExtWindows;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SetCapture(hwnd: *mut c_void) -> *mut c_void;
        fn ReleaseCapture() -> i32;
    }
    // SAFETY: `hwnd` es la ventana viva de `window`; ambas llamadas solo afectan al ratón.
    unsafe {
        if on {
            SetCapture(window.hwnd() as isize as *mut c_void);
        } else {
            ReleaseCapture();
        }
    }
}

#[cfg(not(windows))]
fn capture_mouse(_window: &Window, _on: bool) {}

/// Zonas de la ventana: lienzo arriba a la izquierda, vista de código debajo (si está
/// abierta) y panel de ancho fijo a la derecha. Con `viewport`, el lienzo tiene ese ancho
/// lógico (recortado al espacio disponible) y queda centrado.
struct Layout {
    toolbar: Rect,
    canvas: Rect,
    code: Rect,
    panel: Rect,
}

fn layout(window: &Window, viewport: Option<u32>, arrangement: Arrangement, frac: Fractions) -> Layout {
    let size = window.inner_size();
    let z = boxes((size.width, size.height), window.scale_factor(), viewport, arrangement, frac);
    Layout {
        toolbar: rect(z.toolbar.0, z.toolbar.1, z.toolbar.2, z.toolbar.3),
        canvas: rect(z.canvas.0, z.canvas.1, z.canvas.2, z.canvas.3),
        code: rect(z.code.0, z.code.1, z.code.2, z.code.3),
        panel: rect(z.panel.0, z.panel.1, z.panel.2, z.panel.3),
    }
}

/// Alto (lógico) de la barra de herramientas.
const TOOLBAR_LOGICAL: f64 = 38.0;

fn toolbar_px(scale: f64) -> u32 {
    (TOOLBAR_LOGICAL * scale).round() as u32
}

/// Dónde cae el menú flotante `name`: bajo la barra, con su borde izquierdo en `x_css` (lógico)
/// pero sin salirse de la ventana. Su tamaño (lógico) es el que el propio menú dijo que
/// necesita (`measured`); hasta entonces, una estimación según el menú.
fn popup_px(size: (u32, u32), scale: f64, name: &str, x_css: f64, measured: Option<(f64, f64)>) -> Px {
    let (w_css, h_css) = measured.unwrap_or(match name {
        "file" => (240.0, 120.0),
        "edit" => (260.0, 150.0),
        "tags" => (260.0, 520.0),
        "snippets" => (300.0, 260.0),
        "view" => (240.0, 120.0),
        "search" | "review" => (470.0, 540.0),
        _ => (340.0, 200.0),
    });
    let bar = toolbar_px(scale);
    let w = ((w_css * scale) as u32).min(size.0);
    let h = ((h_css * scale) as u32).min(size.1.saturating_sub(bar));
    let x = ((x_css * scale) as i32).clamp(0, (size.0 - w) as i32);
    (x, bar as i32, w, h)
}

/// Rectángulo `(x, y, ancho, alto)` en píxeles físicos.
type Px = (i32, i32, u32, u32);

/// Cuánto ocupa la vista de código: fracción del alto (debajo) o del ancho (al lado).
#[derive(Clone, Copy, PartialEq, Debug)]
struct Fractions {
    below: f64,
    side: f64,
}

impl Default for Fractions {
    fn default() -> Self {
        Self { below: 0.38, side: 0.45 }
    }
}

/// Ni el lienzo ni el código pueden quedar más pequeños que esto al arrastrar el divisor.
const MIN_FRACTION: f64 = 0.15;
const MAX_FRACTION: f64 = 0.75;
/// Grosor (lógico) de la franja libre entre el lienzo y el código: es lo que se agarra.
const DIVIDER_LOGICAL: f64 = 6.0;

fn divider_px(scale: f64) -> u32 {
    (DIVIDER_LOGICAL * scale).round().max(1.0) as u32
}

/// Cómo se reparte la ventana entre el lienzo y la vista de código.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Arrangement {
    /// Lienzo arriba y código debajo.
    Below,
    /// Lienzo a la izquierda y código a la derecha del lienzo.
    Side,
    CodeOnly,
    CanvasOnly,
}

impl Arrangement {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "below" => Self::Below,
            "side" => Self::Side,
            "code" => Self::CodeOnly,
            "canvas" => Self::CanvasOnly,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::Below => "below",
            Self::Side => "side",
            Self::CodeOnly => "code",
            Self::CanvasOnly => "canvas",
        }
    }

    fn code_visible(self) -> bool {
        self != Self::CanvasOnly
    }

    fn canvas_visible(self) -> bool {
        self != Self::CodeOnly
    }
}

/// Zonas de la ventana en píxeles físicos.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Zones {
    /// Barra de herramientas: arriba, a todo el ancho.
    toolbar: Px,
    canvas: Px,
    code: Px,
    panel: Px,
    /// La franja libre entre lienzo y código (solo cuando se ven los dos).
    divider: Option<Px>,
}

/// Reparte la ventana. El panel ocupa siempre la columna derecha; con `viewport`, el lienzo
/// tiene ese ancho lógico (recortado al espacio que le toca) y queda centrado. Lo que no
/// se ve mide 0. Entre lienzo y código queda una franja libre de la ventana (`divider`).
fn boxes(
    size: (u32, u32),
    scale: f64,
    viewport: Option<u32>,
    arrangement: Arrangement,
    frac: Fractions,
) -> Zones {
    let (width, full_height) = size;
    // Lo de debajo de la barra de herramientas se reparte en `height`, desde `top`.
    let bar = toolbar_px(scale).min(full_height);
    let top = bar as i32;
    let height = full_height - bar;
    let panel_w = ((PANEL_WIDTH_LOGICAL * scale) as u32).min(width / 2);
    let area_w = width - panel_w;
    let gap = divider_px(scale);
    let (canvas_area, code_area, divider): (Px, Px, Option<Px>) = match arrangement {
        Arrangement::Below => {
            let gap = gap.min(height);
            let code_h = ((f64::from(height) * frac.below) as u32).min(height - gap);
            let canvas_h = height - gap - code_h;
            (
                (0, 0, area_w, canvas_h),
                (0, (canvas_h + gap) as i32, area_w, code_h),
                Some((0, canvas_h as i32, area_w, gap)),
            )
        }
        Arrangement::Side => {
            let gap = gap.min(area_w);
            let code_w = ((f64::from(area_w) * frac.side) as u32).min(area_w - gap);
            let canvas_w = area_w - gap - code_w;
            (
                (0, 0, canvas_w, height),
                ((canvas_w + gap) as i32, 0, code_w, height),
                Some((canvas_w as i32, 0, gap, height)),
            )
        }
        Arrangement::CodeOnly => ((0, 0, 0, 0), (0, 0, area_w, height), None),
        Arrangement::CanvasOnly => ((0, 0, area_w, height), (0, 0, 0, 0), None),
    };
    let canvas_w = viewport.map_or(canvas_area.2, |w| ((f64::from(w) * scale) as u32).min(canvas_area.2));
    let x = canvas_area.0 + ((canvas_area.2 - canvas_w) / 2) as i32;
    let shift = |(x, y, w, h): Px| (x, y + top, w, h);
    Zones {
        toolbar: (0, 0, width, bar),
        canvas: shift((x, canvas_area.1, canvas_w, canvas_area.3)),
        code: shift(code_area),
        panel: (area_w as i32, top, panel_w, height),
        divider: divider.map(shift),
    }
}

/// La fracción de código que corresponde a arrastrar el divisor hasta el punto `(x, y)`
/// (el centro de la franja sigue al ratón), o `None` si la disposición no tiene divisor.
fn fraction_at(size: (u32, u32), scale: f64, arrangement: Arrangement, (x, y): (f64, f64)) -> Option<f64> {
    let (width, full_height) = size;
    let bar = toolbar_px(scale).min(full_height);
    let height = full_height - bar;
    let y = y - f64::from(bar);
    let panel_w = ((PANEL_WIDTH_LOGICAL * scale) as u32).min(width / 2);
    let area_w = f64::from(width - panel_w);
    let half = f64::from(divider_px(scale)) / 2.0;
    let f = match arrangement {
        Arrangement::Below => (f64::from(height) - y - half) / f64::from(height).max(1.0),
        Arrangement::Side => (area_w - x - half) / area_w,
        Arrangement::CodeOnly | Arrangement::CanvasOnly => return None,
    };
    Some(f.clamp(MIN_FRACTION, MAX_FRACTION))
}

fn inside((x, y, w, h): Px, (px, py): (f64, f64)) -> bool {
    px >= f64::from(x) && px < f64::from(x) + f64::from(w) && py >= f64::from(y) && py < f64::from(y) + f64::from(h)
}

fn forward(proxy: &EventLoopProxy<FromWebview>, body: &str) {
    match serde_json::from_str::<FromWebview>(body) {
        Ok(msg) => {
            let _ = proxy.send_event(msg);
        }
        Err(e) => eprintln!("mensaje IPC inválido ({e}): {body}"),
    }
}

/// Separa `--lang xx` del resto de argumentos: `(idioma pedido, primer argumento que no es
/// una opción)`.
fn parse_args(args: impl IntoIterator<Item = String>) -> (Option<String>, Option<String>) {
    let mut lang = None;
    let mut path = None;
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        if a == "--lang" {
            lang = args.next();
        } else if let Some(v) = a.strip_prefix("--lang=") {
            lang = Some(v.to_string());
        } else if !a.starts_with("--") && path.is_none() {
            path = Some(a);
        }
    }
    (lang, path)
}

/// Texto del aviso cuando la aplicación no puede arrancar (típicamente, falta el motor web:
/// WebView2 en Windows 10 o WebKitGTK en Linux).
fn startup_failure_text(error: &str) -> String {
    t!("status.startup_failed", error = error)
}

fn main() {
    if let Err(e) = run() {
        // Sin consola (subsistema gráfico) un error de arranque sería invisible.
        MessageDialog::new()
            .set_level(MessageLevel::Error)
            .set_title(t("status.startup_failed_title"))
            .set_description(startup_failure_text(&e.to_string()))
            .show();
        std::process::exit(1);
    }
}

fn run() -> wry::Result<()> {
    // Idioma de la interfaz: --lang, TEMIKTLI_LANG, config.json, el de Windows, inglés.
    let (cli_lang, arg_path) = parse_args(std::env::args().skip(1));
    let config_path = config::storage_path();
    let cfg = config_path.as_deref().map(config::load).unwrap_or_default();
    let env_lang = std::env::var("TEMIKTLI_LANG").ok();
    let lang = i18n::resolve_language(
        cli_lang.as_deref(),
        env_lang.as_deref(),
        cfg.language.as_deref(),
        i18n::system_language().as_deref(),
    );
    i18n::set_language(&lang).expect("idioma incrustado");
    let language_setting = cfg.language.unwrap_or_else(|| "system".into());

    // Documento inicial: el archivo pasado por argumento, o uno nuevo en blanco.
    let arg_path = arg_path.map(PathBuf::from);
    let mut doc = Document::parse(&blank_html());
    let mut path = None;
    let mut status = String::new();
    if let Some(p) = arg_path {
        match std::fs::read(&p).map(String::from_utf8) {
            Ok(Ok(text)) => {
                doc = Document::parse(text.trim_start_matches('\u{feff}'));
                path = Some(p);
            }
            Ok(Err(_)) => status = t!("status.startup_not_utf8", path = p.display()),
            Err(e) => status = t!("status.startup_open_failed", path = p.display(), error = e),
        }
    }

    // Los snippets propios: si el archivo estaba dañado se avisa y se sigue con la lista vacía.
    let snippets_path = snippets::storage_path();
    let loaded = snippets_path.as_deref().map(snippets::load);
    let (user_snippets, snippet_warning) = match loaded {
        Some(l) => (l.snippets, l.warning),
        None => (Vec::new(), None),
    };
    if let Some(warning) = snippet_warning {
        status = format!("⚠ {warning}");
    }

    let shared: SharedRef = Arc::new(Mutex::new(Shared {
        canvas_html: doc.to_canvas_html(),
        site_root: path.as_deref().and_then(Path::parent).map(Path::to_path_buf),
        overrides: HashMap::new(),
    }));

    let event_loop = EventLoopBuilder::<FromWebview>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("Temiktli")
        .with_window_icon(window_icon())
        .with_inner_size(LogicalSize::new(1280.0, 800.0))
        .build(&event_loop)
        .expect("ventana");
    fit_window_to_screen(&window);
    let lay = layout(&window, None, Arrangement::Below, Fractions::default());

    let (canvas_proxy, panel_proxy, code_proxy, toolbar_proxy, popup_proxy) = (
        event_loop.create_proxy(),
        event_loop.create_proxy(),
        event_loop.create_proxy(),
        event_loop.create_proxy(),
        event_loop.create_proxy(),
    );
    let shared_for_protocol = shared.clone();

    let canvas = WebViewBuilder::new()
        .with_bounds(lay.canvas)
        .with_custom_protocol("tk".into(), move |_id, req| serve(&shared_for_protocol, &req))
        .with_url(canvas_url(0))
        .with_initialization_script(inject_js())
        // Nunca salir del documento: enlaces y redirecciones de la página no navegan.
        .with_navigation_handler(|url| url.starts_with(ORIGIN))
        .with_ipc_handler(move |req| forward(&canvas_proxy, req.body()))
        .build_as_child(&window)?;

    let panel = WebViewBuilder::new()
        .with_bounds(lay.panel)
        .with_html(panel_page())
        .with_ipc_handler(move |req| forward(&panel_proxy, req.body()))
        .build_as_child(&window)?;

    let code = WebViewBuilder::new()
        .with_bounds(lay.code)
        .with_html(code_page())
        .with_ipc_handler(move |req| forward(&code_proxy, req.body()))
        .build_as_child(&window)?;

    let toolbar = WebViewBuilder::new()
        .with_bounds(lay.toolbar)
        .with_html(toolbar_page())
        .with_ipc_handler(move |req| forward(&toolbar_proxy, req.body()))
        .build_as_child(&window)?;

    // El último en crearse queda por encima de los demás: es el menú flotante, oculto hasta
    // que se abre uno desde la barra.
    let popup = WebViewBuilder::new()
        .with_bounds(lay.toolbar)
        .with_visible(false)
        .with_html(popup_page())
        .with_ipc_handler(move |req| forward(&popup_proxy, req.body()))
        .build_as_child(&window)?;

    let mut app = App {
        doc,
        history: History::default(),
        selection: None,
        path,
        dirty: false,
        status,
        shared,
        load_version: 0,
        viewport: None,
        theme: "system".into(),
        language: language_setting,
        config_path,
        probed_rules: Vec::new(),
        matched_rules: Vec::new(),
        css_files: Vec::new(),
        active_file: None,
        site_files: Vec::new(),
        files_dirty: true,
        pending_goto: None,
        undo_log: Vec::new(),
        redo_log: Vec::new(),
        code,
        code_open: true,
        arrangement: Arrangement::Below,
        last_layout: None,
        open_menu: None,
        menu_x: 0.0,
        menu_dims: None,
        menu_closed: None,
        clipboard: None,
        user_snippets,
        snippets_path,
        toolbar,
        popup,
        fractions: Fractions::default(),
        over_divider: false,
        dragging: false,
        doc_version: 0,
        review: None,
        auto_review: true,
        span_index: None,
        code_dirty: true,
        code_force: true,
        code_text: String::new(),
        canvas,
        panel,
    };
    app.refresh_files();
    window.set_title(&app.title());

    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(msg) => {
                app.handle(msg);
                window.set_title(&app.title());
                app.relayout(&window);
            }
            Event::WindowEvent {
                event: WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. },
                ..
            } => app.relayout(&window),
            Event::WindowEvent { event: WindowEvent::CursorMoved { position, .. }, .. } => {
                app.pointer_moved(&window, (position.x, position.y));
            }
            Event::WindowEvent {
                event: WindowEvent::MouseInput { state, button: MouseButton::Left, .. },
                ..
            } => app.pointer_button(&window, state == ElementState::Pressed),
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                if app.confirm_discard() {
                    *flow = ControlFlow::Exit;
                }
            }
            _ => {}
        }
    });
}
