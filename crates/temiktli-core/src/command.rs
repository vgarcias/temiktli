//! Patrón Command: toda mutación del documento pasa por aquí, lo que da
//! deshacer/rehacer y un único punto de entrada (también para scripting futuro).

use crate::dom::{Document, valid_attr_name};
use crate::protocol::{NodeId, Patch};
use crate::t;

pub trait Command {
    fn label(&self) -> &str;
    /// Aplica el cambio. Un `Err` significa que no se tocó el documento.
    fn apply(&mut self, doc: &mut Document) -> Result<Vec<Patch>, String>;
    /// Deshace lo aplicado por `apply`.
    fn revert(&mut self, doc: &mut Document) -> Vec<Patch>;
}

#[derive(Default)]
pub struct History {
    undo: Vec<Box<dyn Command>>,
    redo: Vec<Box<dyn Command>>,
}

impl History {
    pub fn execute(&mut self, doc: &mut Document, mut cmd: Box<dyn Command>) -> Result<Vec<Patch>, String> {
        let patches = cmd.apply(doc)?;
        self.undo.push(cmd);
        self.redo.clear(); // una acción nueva invalida el rehacer
        Ok(patches)
    }

    pub fn undo(&mut self, doc: &mut Document) -> Option<(String, Vec<Patch>)> {
        let mut cmd = self.undo.pop()?;
        let patches = cmd.revert(doc);
        let label = cmd.label().to_string();
        self.redo.push(cmd);
        Some((label, patches))
    }

    pub fn redo(&mut self, doc: &mut Document) -> Option<(String, Vec<Patch>)> {
        let mut cmd = self.redo.pop()?;
        // Un comando que ya se aplicó una vez no debería fallar al repetirse.
        let patches = cmd.apply(doc).unwrap_or_default();
        let label = cmd.label().to_string();
        self.undo.push(cmd);
        Some((label, patches))
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

// ---------- ReplaceDocument ----------

/// Sustituye todo el documento por el resultado de parsear `html` (edición desde la vista
/// de código). Aplicar y deshacer intercambian los dos documentos tal cual, así que los
/// ids de cada uno siguen siendo válidos para los comandos anteriores y posteriores, y el
/// texto escrito se conserva literalmente al guardar.
pub struct ReplaceDocument {
    other: Document,
}

impl ReplaceDocument {
    pub fn new(html: &str) -> Self {
        Self { other: Document::parse(html) }
    }
}

impl Command for ReplaceDocument {
    fn label(&self) -> &str {
        "command.edit_code"
    }

    fn apply(&mut self, doc: &mut Document) -> Result<Vec<Patch>, String> {
        std::mem::swap(doc, &mut self.other);
        Ok(vec![Patch::Reload])
    }

    fn revert(&mut self, doc: &mut Document) -> Vec<Patch> {
        std::mem::swap(doc, &mut self.other);
        vec![Patch::Reload]
    }
}

// ---------- SetAttr ----------

pub struct SetAttr {
    id: NodeId,
    name: String,
    new: Option<String>,
    old: Option<Option<String>>, // None = aún no aplicado
}

impl SetAttr {
    pub fn new(id: NodeId, name: String, new: Option<String>) -> Self {
        Self { id, name: name.to_ascii_lowercase(), new, old: None }
    }

    fn patch(&self, value: Option<String>) -> Vec<Patch> {
        vec![Patch::SetAttr { id: self.id, name: self.name.clone(), value }]
    }
}

impl Command for SetAttr {
    fn label(&self) -> &str {
        "command.set_attr"
    }

    fn apply(&mut self, doc: &mut Document) -> Result<Vec<Patch>, String> {
        if !valid_attr_name(&self.name) {
            return Err(t!("command.invalid_attr_name", name = self.name));
        }
        if !doc.contains(self.id) {
            return Err(t("command.element_gone"));
        }
        let prev = doc.set_attr(self.id, &self.name, self.new.clone());
        self.old.get_or_insert(prev);
        Ok(self.patch(self.new.clone()))
    }

    fn revert(&mut self, doc: &mut Document) -> Vec<Patch> {
        let old = self.old.clone().flatten();
        doc.set_attr(self.id, &self.name, old.clone());
        self.patch(old)
    }
}

// ---------- SetText ----------

pub struct SetText {
    id: NodeId,
    new: String,
    old: Option<String>,
}

impl SetText {
    pub fn new(id: NodeId, new: String) -> Self {
        Self { id, new, old: None }
    }
}

impl Command for SetText {
    fn label(&self) -> &str {
        "command.edit_text"
    }

    fn apply(&mut self, doc: &mut Document) -> Result<Vec<Patch>, String> {
        let prev = doc
            .set_text(self.id, &self.new)
            .ok_or_else(|| t("command.text_only_elements"))?;
        self.old.get_or_insert(prev);
        Ok(vec![Patch::SetText { id: self.id, text: self.new.clone() }])
    }

    fn revert(&mut self, doc: &mut Document) -> Vec<Patch> {
        let old = self.old.clone().unwrap_or_default();
        doc.set_text(self.id, &old);
        vec![Patch::SetText { id: self.id, text: old }]
    }
}

// ---------- InsertElement ----------

pub struct InsertElement {
    parent: NodeId,
    index: usize,
    tag: String,
    /// Atributos que sustituyen a los de fábrica del elemento nuevo (p. ej. el `src` real
    /// de una imagen); solo se aplican al crearlo, no al rehacer.
    attrs: Vec<(String, String)>,
    created: Option<NodeId>,
}

impl InsertElement {
    pub fn new(parent: NodeId, index: usize, tag: String) -> Self {
        Self { parent, index, tag, attrs: Vec::new(), created: None }
    }

    /// Como `new`, con atributos propios en el mismo paso de deshacer.
    pub fn with_attrs(parent: NodeId, index: usize, tag: String, attrs: Vec<(String, String)>) -> Self {
        Self { parent, index, tag, attrs, created: None }
    }

    /// Id del elemento creado, una vez aplicado.
    pub fn created(&self) -> Option<NodeId> {
        self.created
    }
}

impl Command for InsertElement {
    fn label(&self) -> &str {
        "command.insert_element"
    }

    fn apply(&mut self, doc: &mut Document) -> Result<Vec<Patch>, String> {
        if let Some((name, _)) = self.attrs.iter().find(|(n, _)| !valid_attr_name(n)) {
            return Err(t!("command.invalid_attr_name", name = name));
        }
        let id = match self.created {
            Some(id) => id, // rehacer: el mismo nodo, con el mismo id
            None => {
                let id = doc
                    .create_element(&self.tag)
                    .ok_or_else(|| t!("command.cannot_insert_tag", tag = self.tag))?;
                for (name, value) in &self.attrs {
                    doc.set_attr(id, name, Some(value.clone()));
                }
                id
            }
        };
        let index = doc
            .insert(self.parent, self.index, id)
            .ok_or_else(|| t("command.cannot_insert_inside"))?;
        self.created = Some(id);
        self.index = index;
        let html = doc.subtree_html(id).ok_or_else(|| t("command.serialize_failed"))?;
        Ok(vec![Patch::Insert { parent: self.parent, index, html }])
    }

    fn revert(&mut self, doc: &mut Document) -> Vec<Patch> {
        match self.created {
            Some(id) => {
                doc.detach(id);
                vec![Patch::Remove { id }]
            }
            None => Vec::new(),
        }
    }
}

// ---------- MoveNode ----------

/// Mueve un elemento a otro sitio del documento. El destino ya viene validado
/// (`Document::resolve_drop`); `index` se cuenta con el elemento todavía en su sitio.
pub struct MoveNode {
    id: NodeId,
    parent: NodeId,
    index: usize,
    /// Dónde estaba antes de aplicar.
    from: Option<(NodeId, usize)>,
}

impl MoveNode {
    pub fn new(id: NodeId, parent: NodeId, index: usize) -> Self {
        Self { id, parent, index, from: None }
    }

    fn moved_patches(&self, doc: &Document, at: (NodeId, usize)) -> Vec<Patch> {
        match doc.subtree_html(self.id) {
            Some(html) => vec![Patch::Remove { id: self.id }, Patch::Insert { parent: at.0, index: at.1, html }],
            None => vec![Patch::Reload],
        }
    }
}

impl Command for MoveNode {
    fn label(&self) -> &str {
        "command.move_element"
    }

    fn apply(&mut self, doc: &mut Document) -> Result<Vec<Patch>, String> {
        if doc.tag(self.id).is_none() {
            return Err(t("command.element_gone"));
        }
        let (old_parent, old_index) = doc.detach(self.id).ok_or_else(|| t("command.no_parent"))?;
        // Al quitarlo de delante, los hermanos siguientes se corren una posición.
        let index = if old_parent == self.parent && old_index < self.index { self.index - 1 } else { self.index };
        match doc.insert(self.parent, index, self.id) {
            Some(real) => {
                self.from = Some((old_parent, old_index));
                Ok(self.moved_patches(doc, (self.parent, real)))
            }
            None => {
                doc.insert(old_parent, old_index, self.id); // vuelve a su sitio: nada cambió
                Err(t("command.cannot_drop_here"))
            }
        }
    }

    fn revert(&mut self, doc: &mut Document) -> Vec<Patch> {
        let Some(from) = self.from else { return Vec::new() };
        doc.detach(self.id);
        doc.insert(from.0, from.1, self.id);
        self.moved_patches(doc, from)
    }
}

// ---------- DeleteNode ----------

pub struct DeleteNode {
    id: NodeId,
    saved: Option<(NodeId, usize)>,
}

impl DeleteNode {
    pub fn new(id: NodeId) -> Self {
        Self { id, saved: None }
    }
}

impl Command for DeleteNode {
    fn label(&self) -> &str {
        "command.delete_element"
    }

    fn apply(&mut self, doc: &mut Document) -> Result<Vec<Patch>, String> {
        match doc.tag(self.id).as_deref() {
            None => return Err(t("command.element_gone")),
            Some("html" | "head" | "body") => return Err(t("command.cannot_delete_root")),
            Some(_) => {}
        }
        self.saved = Some(doc.detach(self.id).ok_or_else(|| t("command.no_parent"))?);
        Ok(vec![Patch::Remove { id: self.id }])
    }

    fn revert(&mut self, doc: &mut Document) -> Vec<Patch> {
        let Some((parent, index)) = self.saved else { return Vec::new() };
        doc.insert(parent, index, self.id);
        match doc.subtree_html(self.id) {
            Some(html) => vec![Patch::Insert { parent, index, html }],
            None => Vec::new(),
        }
    }
}
