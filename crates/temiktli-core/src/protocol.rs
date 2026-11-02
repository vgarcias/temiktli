//! Mensajes entre los webviews (lienzo y panel) y Rust.

use crate::css::RuleView;
use crate::lint::ReviewCounts;
use serde::{Deserialize, Serialize};

pub type NodeId = u64;

/// JS → Rust. Lo envían tanto el lienzo como el panel.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum FromWebview {
    /// Un webview terminó de cargar y pide el estado actual.
    Ready,
    Select { id: NodeId },
    EditText { id: NodeId, text: String },
    SetAttr { id: NodeId, name: String, value: Option<String> },
    /// Inserta un elemento nuevo relativo a la selección.
    Insert { tag: String },
    /// Borra el elemento seleccionado.
    Delete,
    Undo,
    Redo,
    Open,
    Save,
    SaveAs,
    /// Arrastrando un elemento: ¿se puede soltar `id` en `place` ("before", "after" o "inside")
    /// de `target`? Rust responde con `InsertMark` (dónde caería) y `DropState`.
    DragOver { id: NodeId, target: NodeId, place: String },
    /// Se soltó `id` en `place` de `target`: se mueve si las reglas de anidación lo permiten.
    MoveElement { id: NodeId, target: NodeId, place: String },
    /// Terminó el arrastre (suelte o cancelación): se restaura el cursor de inserción.
    DragEnd,
    /// Cierra el documento actual (pregunta por cambios sin guardar) y deja uno en blanco.
    CloseDocument,
    /// Ancho del lienzo en píxeles lógicos para probar diseño responsivo; `None` = todo el espacio.
    SetViewport { width: Option<u32> },
    /// Esquema de color que ve la página (`prefers-color-scheme`): "system", "light" o "dark".
    SetTheme { theme: String },
    /// Idioma de la interfaz elegido en el menú: "system" o un código de `i18n::LANGUAGES`.
    /// Se guarda en la configuración y se aplica al reiniciar.
    SetLanguage { lang: String },
    /// Cambia (`Some`) o quita (`None`) una declaración de una regla de un `<style>`.
    SetDecl { style: NodeId, rule: usize, name: String, value: Option<String> },
    /// Desde la vista de código: el usuario escribió este HTML completo.
    SetSource { html: String },
    /// Muestra u oculta la vista de código.
    SetCodeView { open: bool },
    /// Disposición del lienzo y la vista de código: "below", "side", "code" o "canvas".
    SetLayout { layout: String },
    /// Abre otro documento HTML del sitio (ruta relativa a la carpeta del documento).
    OpenPath { path: String },
    /// Abre un `.css` del sitio como pestaña de la vista de código.
    OpenCss { path: String },
    /// Cambia de pestaña en la vista de código: `None` es el documento HTML.
    ActivateFile { path: Option<String> },
    /// Cierra la pestaña de un `.css` (solo si no tiene cambios sin guardar).
    CloseFile { path: String },
    /// Desde la vista de código: el usuario editó este `.css`.
    SetFileText { path: String, text: String },
    /// Cambia (`Some`) o quita (`None`) una declaración de una regla de un `.css` externo.
    SetFileDecl { path: String, rule: usize, name: String, value: Option<String> },
    /// Añade `<link rel="stylesheet">` al `<head>` del documento.
    LinkCss { path: String },
    /// Inserta una imagen del sitio (`<img src alt>`) relativa a la selección.
    InsertImage { path: String },
    /// Crea un archivo nuevo (.css, .html o .htm) en la carpeta del sitio y lo abre.
    NewFile { path: String },
    /// Envía a la Papelera un archivo o carpeta del sitio (tras confirmarlo en un diálogo).
    DeletePath { path: String },
    /// Renombra un archivo o carpeta del sitio; `new_name` es solo el nombre, sin carpetas.
    RenamePath { path: String, new_name: String },
    /// Vuelve a leer la carpeta del sitio.
    RefreshFiles,
    /// El panel pide el texto de los archivos editables del sitio para buscar en ellos.
    SearchSite,
    /// Copia el elemento seleccionado (con su contenido) al portapapeles del editor.
    Copy,
    /// Pega lo copiado justo después del elemento seleccionado.
    Paste,
    /// La barra de herramientas abre un menú desplegable (`insert`, `view`, `search` o
    /// `review`) bajo el botón cuyo borde izquierdo está en `x` (píxeles lógicos).
    OpenMenu { name: String, x: f64 },
    /// Cierra el menú desplegable.
    CloseMenu,
    /// El menú desplegable mide lo que necesita (píxeles lógicos): se ajusta la ventana.
    MenuSize { width: f64, height: f64 },
    /// Desde Propiedades: sustituye el interior del elemento por este HTML.
    SetInner { id: NodeId, html: String },
    /// El panel pide revisar el documento (validación y accesibilidad).
    Review,
    /// Guarda el elemento seleccionado como snippet propio con este nombre.
    SaveSnippet { name: String },
    /// Inserta el snippet número `index` de la lista que se mostró (los de serie y luego los propios).
    InsertSnippet { index: usize },
    /// Borra el snippet propio número `index` de esa misma lista. Los de serie no se borran.
    DeleteSnippet { index: usize },
    /// Limpia el HTML del elemento seleccionado (o de todo el cuerpo si no hay selección):
    /// restos de Word, atributos de estilo, etiquetas vacías y formato semántico.
    CleanHtml,
    /// Indenta todo el HTML del documento con tabulaciones (una acción que se puede deshacer).
    IndentHtml,
    /// Reduce todo el HTML del documento: sin comentarios ni espacios sobrantes (se puede deshacer).
    MinifyHtml,
    /// Activa o desactiva la revisión automática al guardar.
    SetAutoReview { on: bool },
    /// Aplica el arreglo de un clic del aviso número `index` de la última revisión.
    FixIssue { index: usize },
    /// Respuesta del lienzo a la comprobación de contraste: los textos que no llegan al
    /// mínimo, cuántos se comprobaron y cuántos no se pudieron (fondo con imagen, etc.).
    Contrast { items: Vec<ContrastItem>, checked: usize, skipped: usize },
    /// Desde la vista de código: el cursor está en esta posición (unidades UTF-16) del HTML
    /// mostrado. Solo se envía cuando el texto coincide con el del documento.
    CursorAt { offset: usize },
    /// Aplica un reemplazo calculado en el panel sobre varios archivos. `count` es el total
    /// de coincidencias (para confirmarlo con el usuario).
    ReplaceInSite { count: usize, files: Vec<SiteEdit> },
    /// Abre `path` (.html o .css) y selecciona `len` caracteres desde la línea y columna
    /// dadas (ambas desde 0, en unidades de UTF-16 como las de JavaScript).
    GoTo { path: String, line: usize, col: usize, len: usize },
    /// Respuesta del lienzo: qué reglas (en el orden enviado) afectan al elemento `id`.
    Matches { id: NodeId, rules: Vec<bool> },
}

/// Un texto cuyo contraste con su fondo no llega al mínimo, medido en el lienzo.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ContrastItem {
    pub id: NodeId,
    pub ratio: f64,
    pub fg: String,
    pub bg: String,
    /// Texto grande (≥ 24 px, o ≥ 18,66 px en negrita): el mínimo es 3:1 en vez de 4,5:1.
    pub large: bool,
}

/// Texto nuevo de un archivo del sitio. `from_len` (en unidades UTF-16) es el largo del texto
/// sobre el que se calculó: si el archivo cambió desde entonces, no se aplica.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SiteEdit {
    pub path: String,
    pub from_len: usize,
    pub text: String,
}

/// Rust → lienzo.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type")]
pub enum Patch {
    Highlight { id: Option<NodeId> },
    /// Dónde caería el próximo elemento: dentro de `parent`, antes del hijo número `index`
    /// (mismo criterio que `Insert`). `None` = no hay un sitio claro y no se dibuja nada.
    InsertMark { parent: Option<NodeId>, index: usize },
    /// Respuesta a `DragOver`: `ok` si se puede soltar ahí; si no, `why` dice por qué.
    DropState { ok: bool, why: Option<String> },
    SetAttr { id: NodeId, name: String, value: Option<String> },
    SetText { id: NodeId, text: String },
    Insert { parent: NodeId, index: usize, html: String },
    Remove { id: NodeId },
    /// El documento cambió por completo: hay que recargar el lienzo.
    Reload,
}

/// Un tramo de la ruta del elemento seleccionado (`div.hero`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Crumb {
    pub id: NodeId,
    pub label: String,
}

/// Rust → panel: elemento seleccionado.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SelectionInfo {
    pub id: NodeId,
    pub tag: String,
    /// Ruta estilo Dreamweaver: `html > body > div.hero > h1`.
    pub path: String,
    /// La misma ruta paso a paso, de `html` al elemento, para poder seleccionar cada tramo.
    pub crumbs: Vec<Crumb>,
    /// Atributos editables, sin el marcador interno del editor.
    pub attrs: Vec<(String, String)>,
    /// `Some` solo si el elemento contiene únicamente texto.
    pub text: Option<String>,
    /// El interior en HTML, para elementos con elementos dentro (donde `text` es `None`).
    pub inner_html: Option<String>,
    /// Tiene interior editable pero es demasiado grande para ofrecerlo aquí.
    pub inner_large: bool,
    pub can_delete: bool,
}

/// Entrada de la carpeta del sitio. `path` es relativa a la carpeta del documento y usa `/`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct FileEntry {
    pub path: String,
    pub is_dir: bool,
}

/// Un `.css` abierto como pestaña de la vista de código.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OpenCss {
    pub path: String,
    pub dirty: bool,
}

/// Rust → panel: estado completo. Se reenvía entero tras cada acción.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PanelState {
    pub selection: Option<SelectionInfo>,
    pub can_undo: bool,
    pub can_redo: bool,
    /// Hay un elemento copiado que se puede pegar.
    pub can_paste: bool,
    /// Resultado de la última revisión, mientras siga vigente (el documento no ha cambiado).
    pub review: Option<ReviewCounts>,
    pub dirty: bool,
    pub file_name: String,
    /// Mensaje breve para la barra de estado (errores, avisos).
    pub status: String,
    pub viewport: Option<u32>,
    pub theme: String,
    /// Idioma configurado para marcarlo en el menú: "system" o un código de idioma.
    pub language: String,
    /// Reglas CSS que afectan al elemento seleccionado, en orden de aparición.
    pub rules: Vec<RuleView>,
    pub code_open: bool,
    /// Disposición del lienzo y la vista de código: "below", "side", "code" o "canvas".
    pub layout: String,
    /// Contenido de la carpeta del sitio; `None` si no cambió desde el último envío.
    pub files: Option<Vec<FileEntry>>,
    /// El documento tiene ruta en disco, así que hay carpeta que mostrar.
    pub has_folder: bool,
    pub css_open: Vec<OpenCss>,
    /// Pestaña activa de la vista de código (`None` = el documento HTML).
    pub active_file: Option<String>,
}
