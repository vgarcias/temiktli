//! Núcleo de Temiktli: documento HTML5 editable con `NodeId` estables, comandos con
//! deshacer/rehacer y el protocolo de mensajes con los webviews.

mod clean;
mod command;
mod css;
mod dom;
pub mod i18n;
mod lint;
mod pretty;
mod protocol;
mod reformat;
mod source;
mod spanparse;
mod tags;

pub use clean::{CleanReport, clean_fragment};
pub use command::{Command, DeleteNode, History, InsertElement, MoveNode, ReplaceDocument, SetAttr, SetText};
pub use css::{DeclView, RuleView, Sheet, parse_rules, rule_views_for, set_decl};
pub use dom::{Document, ID_ATTR, is_void, valid_attr_name};
pub use i18n::{t, tn, tnr, tr};
pub use tags::{CATEGORIES as TAG_CATEGORIES, Category as TagCategory, is_insertable, menu_json as tag_menu_json};
pub use lint::{Category, Fix, FixAction, Issue, ReviewCounts, Severity};
pub use reformat::{indent_html, minify_html};
pub use protocol::{ContrastItem, FileEntry, FromWebview, NodeId, OpenCss, PanelState, Patch, SelectionInfo, SiteEdit};
pub use source::{SaveMode, SavedHtml, SourceIndex, fragment_elements};

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"<!DOCTYPE html><html><body><div class="hero big"><h1>Hola</h1><p>Texto <a href="#">enlace</a></p></div></body></html>"##;

    fn id_of_tag(doc: &Document, tag: &str) -> NodeId {
        (1..100).find(|&i| doc.tag(i).as_deref() == Some(tag)).unwrap()
    }

    #[test]
    fn assigns_ids_and_paths() {
        let d = Document::parse(SAMPLE);
        let h1 = id_of_tag(&d, "h1");
        assert_eq!(d.path(h1).unwrap(), "html > body > div.hero > h1");
    }

    #[test]
    fn maps_elements_to_source_positions_and_back() {
        let d = Document::parse(SAMPLE);
        let html = d.to_saved_html().html;
        let (h1, a, div) = (id_of_tag(&d, "h1"), id_of_tag(&d, "a"), id_of_tag(&d, "div"));
        let r = d.tag_range(&html, h1).unwrap();
        assert_eq!(&html[r.clone()], "<h1>");
        assert_eq!(&html[d.tag_range(&html, div).unwrap()], r#"<div class="hero big">"#);
        // Dentro del texto de h1: h1. Dentro de «enlace»: a. Entre elementos: el contenedor.
        assert_eq!(d.element_at(&html, r.end + 1), Some(h1));
        assert_eq!(d.element_at(&html, html.find("enlace").unwrap() + 2), Some(a));
        assert_eq!(d.element_at(&html, html.find("Texto").unwrap()), Some(id_of_tag(&d, "p")));
        assert_eq!(d.element_at(&html, html.find("<div").unwrap() + 1), Some(div));
        // Fuera de cualquier elemento con etiqueta propia (el doctype).
        assert_eq!(d.element_at(&html, 2), None);
        // Un texto que no es el del documento no se interpreta.
        assert_eq!(d.element_at("<p>otro</p>", 1), None);
    }

    const MIXED: &str = "<!DOCTYPE html><html><body>\n  <h2 class=\"t\">A day at <em>Riviera</em> Maya</h2>\n  <p>Fuera</p>\n</body></html>";

    #[test]
    fn selection_offers_inner_html_only_when_plain_text_does_not_fit() {
        let d = Document::parse(MIXED);
        let h2 = d.selection_info(id_of_tag(&d, "h2")).unwrap();
        assert_eq!(h2.text, None);
        assert_eq!(h2.inner_html.as_deref(), Some("A day at <em>Riviera</em> Maya"));
        assert!(!h2.inner_html.as_deref().unwrap().contains(ID_ATTR));
        // Un elemento de solo texto usa el cuadro Texto, no este.
        let p = d.selection_info(id_of_tag(&d, "p")).unwrap();
        assert!(p.text.is_some() && p.inner_html.is_none());
        // El cuerpo es demasiado grande y delicado: se edita en el código.
        let body = d.selection_info(id_of_tag(&d, "body")).unwrap();
        assert!(body.inner_html.is_none());
        // Interior enorme: no se ofrece, pero se avisa.
        let big = format!("<div><b>x</b>{}</div>", "y".repeat(25_000));
        let d = Document::parse(&big);
        let div = d.selection_info(id_of_tag(&d, "div")).unwrap();
        assert!(div.inner_html.is_none() && div.inner_large);
    }

    #[test]
    fn replacing_the_inner_html_changes_only_that_element_and_keeps_the_format() {
        let d = Document::parse(MIXED);
        let h2 = id_of_tag(&d, "h2");
        let html = d.to_saved_html().html;
        let new = d.with_inner_html(&html, h2, "Un día en <strong>Riviera</strong>").unwrap();
        assert_eq!(
            new,
            "<!DOCTYPE html><html><body>\n  <h2 class=\"t\">Un día en <strong>Riviera</strong></h2>\n  <p>Fuera</p>\n</body></html>"
        );
        // Quedarse vacío también vale.
        assert!(d.with_inner_html(&html, h2, "").unwrap().contains("<h2 class=\"t\"></h2>"));
        // Elementos sin interior no se editan.
        let d2 = Document::parse("<p>a<br>b</p>");
        let br = id_of_tag(&d2, "br");
        assert!(d2.with_inner_html(&d2.to_saved_html().html, br, "x").is_err());
    }

    #[test]
    fn copy_takes_the_original_text_and_paste_inserts_it_after_the_target() {
        let html = "<!DOCTYPE html><html><body>\n  <ul>\n    <li class=\"a\">Uno <b>x</b></li>\n    <li>Dos</li>\n  </ul>\n  <p>Fin</p>\n</body></html>";
        let d = Document::parse(html);
        let saved = d.to_saved_html().html;
        let li = id_of_tag(&d, "li");
        let (text, elements) = d.outer_source(&saved, li).unwrap();
        assert_eq!(text, "<li class=\"a\">Uno <b>x</b></li>");
        assert_eq!(elements, 2); // el li y su <b>

        // Pegar tras el <p>: justo detrás de su cierre, sin tocar el resto del formato.
        let p = id_of_tag(&d, "p");
        let new = d.with_pasted(&saved, p, &text, elements).unwrap();
        assert!(new.contains("<p>Fin</p><li class=\"a\">Uno <b>x</b></li>\n</body>"), "{new}");
        // Pegar tras un <li>: se queda como hermano suyo dentro de la lista.
        let new = d.with_pasted(&saved, li, &text, elements).unwrap();
        assert_eq!(Document::parse(&new).element_tree().len(), d.element_tree().len() + 2);
        // Con el cuerpo seleccionado, se pega al final del cuerpo.
        let body = id_of_tag(&d, "body");
        let new = d.with_pasted(&saved, body, "<hr>", 1).unwrap();
        assert!(new.contains("</p>\n<hr></body>"), "{new}");
        // No hay dónde pegar si se selecciona <html> o <head>.
        assert!(d.with_pasted(&saved, id_of_tag(&d, "html"), "<hr>", 1).is_err());
    }

    #[test]
    fn paste_is_refused_when_the_browser_would_reorganize_it() {
        let html = "<!DOCTYPE html><html><body><div id=\"d\"><p>x</p></div></body></html>";
        let d = Document::parse(html);
        let saved = d.to_saved_html().html;
        let p = id_of_tag(&d, "p");
        // Una celda fuera de una tabla la descarta el parser: el recuento no cuadra.
        assert!(d.with_pasted(&saved, p, "<td>celda</td>", 1).is_err());
        // Cierres sueltos: rechazados antes de pegar.
        assert!(d.with_pasted(&saved, p, "</div><p>otro</p>", 1).is_err());
    }

    #[test]
    fn rejects_html_that_would_break_the_surrounding_document() {
        let d = Document::parse(MIXED);
        let h2 = id_of_tag(&d, "h2");
        let html = d.to_saved_html().html;
        // Un cierre sin apertura cerraría el h2 antes de tiempo.
        let e = d.with_inner_html(&html, h2, "texto</h2><p>intruso").unwrap_err();
        assert!(e.contains("</h2>"), "{e}");
        let e = d.with_inner_html(&html, h2, "a</div>b").unwrap_err();
        assert!(e.contains("</div>"), "{e}");
        // Un script sin cerrar se tragaría el resto del archivo.
        let e = d.with_inner_html(&html, h2, "<script>alert(1)").unwrap_err();
        assert!(e.contains("script"), "{e}");
        // Etiquetas bien balanceadas, incluso anidadas o autocerradas, sí pasan.
        assert!(d.with_inner_html(&html, h2, "<b><i>x</i></b><br><img src=a.png alt=''>").is_ok());
        // Y un script completo también.
        assert!(d.with_inner_html(&html, h2, "<script>var a = 1 < 2;</script>").is_ok());
    }

    #[test]
    fn structural_key_finds_the_same_element_in_a_replaced_document() {
        let d = Document::parse(SAMPLE);
        let a = id_of_tag(&d, "a");
        let key = d.structural_key(a).unwrap();
        // Mismo árbol con otro texto: ids nuevos, misma posición.
        let edited = Document::parse(&SAMPLE.replace("Texto", "Otro texto más largo"));
        let found = edited.node_at_key(&key).unwrap();
        assert_eq!(edited.tag(found).as_deref(), Some("a"));
        // Un hermano nuevo después no mueve nada: sigue siendo el mismo enlace.
        let after = Document::parse(&SAMPLE.replace("</p>", "</p><h2>x</h2>"));
        assert_eq!(after.tag(after.node_at_key(&key).unwrap()).as_deref(), Some("a"));
        // Uno nuevo antes desplaza las posiciones: la clave ya no apunta a nada.
        let before = Document::parse(&SAMPLE.replace("<h1>", "<h2>x</h2><h1>"));
        assert_eq!(before.node_at_key(&key), None);
        // Una posición que no existe da None.
        assert_eq!(d.node_at_key(&[0, 9, 9]), None);
        assert_eq!(d.node_at_key(&[]), None);
    }

    #[test]
    fn canvas_html_has_ids_clean_html_does_not() {
        let d = Document::parse(SAMPLE);
        assert!(d.to_canvas_html().contains(ID_ATTR));
        let clean = d.to_clean_html();
        assert!(!clean.contains(ID_ATTR));
        assert!(clean.contains("<h1>Hola</h1>"));
    }

    #[test]
    fn attrs_hide_internal_marker() {
        let d = Document::parse(SAMPLE);
        let div = id_of_tag(&d, "div");
        assert_eq!(d.attrs(div), vec![("class".to_string(), "hero big".to_string())]);
    }

    #[test]
    fn text_only_for_text_only_elements() {
        let d = Document::parse(SAMPLE);
        assert_eq!(d.text(id_of_tag(&d, "h1")).as_deref(), Some("Hola"));
        assert_eq!(d.text(id_of_tag(&d, "p")), None); // tiene un <a> dentro
    }

    #[test]
    fn set_attr_and_undo_redo() {
        let mut d = Document::parse(SAMPLE);
        let mut h = History::default();
        let div = id_of_tag(&d, "div");

        let p = h.execute(&mut d, Box::new(SetAttr::new(div, "id".into(), Some("main".into())))).unwrap();
        assert_eq!(p, vec![Patch::SetAttr { id: div, name: "id".into(), value: Some("main".into()) }]);
        assert_eq!(d.attr(div, "id").as_deref(), Some("main"));

        h.undo(&mut d).unwrap();
        assert_eq!(d.attr(div, "id"), None);
        h.redo(&mut d).unwrap();
        assert_eq!(d.attr(div, "id").as_deref(), Some("main"));
    }

    #[test]
    fn set_attr_rejects_bad_names_and_marker() {
        let mut d = Document::parse(SAMPLE);
        let mut h = History::default();
        let div = id_of_tag(&d, "div");
        for bad in ["", "1x", "a b", "on\"click", ID_ATTR] {
            let r = h.execute(&mut d, Box::new(SetAttr::new(div, bad.into(), Some("x".into()))));
            assert!(r.is_err(), "debería rechazar {bad:?}");
        }
        assert!(!h.can_undo());
    }

    #[test]
    fn set_attr_removal_is_undoable() {
        let mut d = Document::parse(SAMPLE);
        let mut h = History::default();
        let div = id_of_tag(&d, "div");
        h.execute(&mut d, Box::new(SetAttr::new(div, "class".into(), None))).unwrap();
        assert_eq!(d.attr(div, "class"), None);
        h.undo(&mut d).unwrap();
        assert_eq!(d.attr(div, "class").as_deref(), Some("hero big"));
    }

    #[test]
    fn set_text_and_undo() {
        let mut d = Document::parse(SAMPLE);
        let mut h = History::default();
        let h1 = id_of_tag(&d, "h1");
        h.execute(&mut d, Box::new(SetText::new(h1, "Adiós".into()))).unwrap();
        assert!(d.to_clean_html().contains("<h1>Adiós</h1>"));
        h.undo(&mut d).unwrap();
        assert!(d.to_clean_html().contains("<h1>Hola</h1>"));
    }

    #[test]
    fn set_text_rejected_on_mixed_content() {
        let mut d = Document::parse(SAMPLE);
        let mut h = History::default();
        let p = id_of_tag(&d, "p");
        assert!(h.execute(&mut d, Box::new(SetText::new(p, "x".into()))).is_err());
    }

    #[test]
    fn insert_undo_redo_keeps_same_id() {
        let mut d = Document::parse(SAMPLE);
        let mut h = History::default();
        let div = id_of_tag(&d, "div");
        let at = d.child_count(div);

        let patches = h.execute(&mut d, Box::new(InsertElement::new(div, at, "h2".into()))).unwrap();
        let Patch::Insert { html, index, .. } = &patches[0] else { panic!("esperaba Insert") };
        assert_eq!(*index, at);
        assert!(html.contains("<h2") && html.contains(ID_ATTR));
        let new_id = (1..200).find(|&i| d.tag(i).as_deref() == Some("h2")).unwrap();
        assert_eq!(d.child_count(div), at + 1);

        h.undo(&mut d).unwrap();
        assert_eq!(d.child_count(div), at);
        h.redo(&mut d).unwrap();
        assert_eq!(d.child_count(div), at + 1);
        assert_eq!(d.parent_and_index(new_id), Some((div, at)));
    }

    #[test]
    fn insert_list_gives_ids_to_children() {
        let mut d = Document::parse(SAMPLE);
        let mut h = History::default();
        let div = id_of_tag(&d, "div");
        h.execute(&mut d, Box::new(InsertElement::new(div, 0, "ul".into()))).unwrap();
        assert_eq!((1..200).filter(|&i| d.tag(i).as_deref() == Some("li")).count(), 2);
    }

    /// Una página con un sitio válido para cada etiqueta que necesita padre concreto.
    const CONTEXT: &str = r##"<!DOCTYPE html><html lang="es"><head><meta charset="utf-8"><title>T</title></head><body>
<main id="main"><p id="p">Texto</p>
<ul id="ul"><li>uno</li></ul><dl id="dl"><dt>t</dt><dd>d</dd></dl>
<table id="table"><thead id="thead"><tr><th>H</th></tr></thead><tbody id="tbody"><tr id="tr"><td id="td">c</td></tr></tbody></table>
<select id="select"><option>a</option></select><fieldset id="fieldset"><legend>l</legend></fieldset>
<details id="details"><summary>s</summary></details><figure id="figure"><img src="a.png" alt=""></figure>
<picture id="picture"><img src="a.png" alt=""></picture><video id="video"></video><audio id="audio"></audio>
<object id="object"></object><map name="m" id="map"><area alt="" shape="rect" coords="0,0,1,1"></map>
<table id="t2"><colgroup id="colgroup"><col></colgroup><tbody><tr><td>x</td></tr></tbody></table></main></body></html>"##;

    /// Dónde seleccionar para insertar cada etiqueta que depende de un padre.
    fn context_for(tag: &str) -> &'static str {
        match tag {
            "li" => "ul",
            "dt" | "dd" => "dl",
            "option" => "select",
            "optgroup" => "select",
            "tr" => "tbody",
            "th" | "td" => "tr",
            "thead" | "tbody" | "tfoot" | "caption" | "colgroup" => "table",
            "col" => "colgroup",
            "legend" => "fieldset",
            "summary" => "details",
            "figcaption" => "figure",
            "source" => "picture",
            "track" => "video",
            "param" => "object",
            "area" => "map",
            _ => "main",
        }
    }

    #[test]
    fn every_tag_in_the_catalog_survives_saving_and_reopening_unchanged() {
        let mut problems = Vec::new();
        for category_def in TAG_CATEGORIES {
            let category = category_def.name;
            for tag in category_def.all_tags() {
                let mut d = Document::parse(CONTEXT);
                let sel = (1..600).find(|&n| d.attr(n, "id").as_deref() == Some(context_for(tag))).unwrap();
                let spot = d.insertion_point(tag, Some(sel));
                if matches!(tag, "html" | "head" | "body" | "title") {
                    assert!(spot.is_err(), "<{tag}> debería rechazarse");
                    continue;
                }
                let (parent, index) = match spot {
                    Ok(s) => s,
                    Err(e) => {
                        problems.push(format!("[{category}] <{tag}>: sin sitio: {e}"));
                        continue;
                    }
                };
                let mut h = History::default();
                if let Err(e) = h.execute(&mut d, Box::new(InsertElement::new(parent, index, tag.into()))) {
                    problems.push(format!("[{category}] <{tag}>: no se pudo crear: {e}"));
                    continue;
                }
                // Lo guardado, vuelto a leer, tiene que ser exactamente lo que había: si el
                // navegador reorganizara algo, aparecería aquí como un guardado «normalizado».
                let saved = d.to_saved_html();
                if let SaveMode::Normalized(why) = saved.mode {
                    problems.push(format!("[{category}] <{tag}>: el guardado se reformateó ({why})"));
                    continue;
                }
                if Document::parse(&saved.html).fingerprint_loose() != d.fingerprint_loose() {
                    problems.push(format!("[{category}] <{tag}>: al reabrirlo no es lo mismo"));
                }
            }
        }
        assert!(problems.is_empty(), "\n{}", problems.join("\n"));
    }

    #[test]
    fn title_is_inserted_when_the_head_has_none() {
        let mut d = Document::parse("<!DOCTYPE html><html><head></head><body><p>x</p></body></html>");
        let (parent, index) = d.insertion_point("title", None).unwrap();
        let mut h = History::default();
        h.execute(&mut d, Box::new(InsertElement::new(parent, index, "title".into()))).unwrap();
        assert!(d.to_saved_html().html.contains("<title>Page title</title>"));
        // Ahora ya tiene uno.
        assert!(d.insertion_point("title", None).is_err());
    }

    #[test]
    fn cannot_insert_unknown_tag_or_inside_void() {
        let mut d = Document::parse(SAMPLE);
        let mut h = History::default();
        let div = id_of_tag(&d, "div");
        // Fuera del catálogo (obsoleta) o ya existente en el documento: no se crea.
        assert!(h.execute(&mut d, Box::new(InsertElement::new(div, 0, "marquee".into()))).is_err());
        assert!(h.execute(&mut d, Box::new(InsertElement::new(div, 0, "body".into()))).is_err());

        h.execute(&mut d, Box::new(InsertElement::new(div, 0, "img".into()))).unwrap();
        let img = id_of_tag(&d, "img");
        assert!(h.execute(&mut d, Box::new(InsertElement::new(img, 0, "p".into()))).is_err());
    }

    #[test]
    fn delete_and_undo_restores_position() {
        let mut d = Document::parse(SAMPLE);
        let mut h = History::default();
        let h1 = id_of_tag(&d, "h1");
        let before = d.parent_and_index(h1).unwrap();

        let p = h.execute(&mut d, Box::new(DeleteNode::new(h1))).unwrap();
        assert_eq!(p, vec![Patch::Remove { id: h1 }]);
        assert!(!d.to_clean_html().contains("<h1>"));

        let back = h.undo(&mut d).unwrap().1;
        assert!(matches!(back[0], Patch::Insert { .. }));
        assert_eq!(d.parent_and_index(h1), Some(before));
        assert!(d.to_clean_html().contains("<h1>Hola</h1>"));
    }

    #[test]
    fn cannot_delete_structural_elements() {
        let mut d = Document::parse(SAMPLE);
        let mut h = History::default();
        for tag in ["html", "head", "body"] {
            let id = id_of_tag(&d, tag);
            assert!(h.execute(&mut d, Box::new(DeleteNode::new(id))).is_err(), "{tag}");
        }
    }

    #[test]
    fn new_action_clears_redo() {
        let mut d = Document::parse(SAMPLE);
        let mut h = History::default();
        let div = id_of_tag(&d, "div");
        h.execute(&mut d, Box::new(SetAttr::new(div, "id".into(), Some("a".into())))).unwrap();
        h.undo(&mut d).unwrap();
        assert!(h.can_redo());
        h.execute(&mut d, Box::new(SetAttr::new(div, "id".into(), Some("b".into())))).unwrap();
        assert!(!h.can_redo());
    }

    #[test]
    fn selection_info_flags_structural_elements() {
        let d = Document::parse(SAMPLE);
        assert!(!d.selection_info(id_of_tag(&d, "body")).unwrap().can_delete);
        assert!(d.selection_info(id_of_tag(&d, "h1")).unwrap().can_delete);
    }

    #[test]
    fn every_step_of_the_path_can_be_selected() {
        let d = Document::parse("<html><body><div class=\"hero big\"><section><h1>T</h1></section></div></body></html>");
        let h1 = id_of_tag(&d, "h1");
        let info = d.selection_info(h1).unwrap();
        // La ruta de texto sigue igual que antes.
        assert_eq!(info.path, "html > body > div.hero > section > h1");
        // Un tramo por elemento, de html al seleccionado, cada uno con el id de su elemento.
        let labels: Vec<&str> = info.crumbs.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["html", "body", "div.hero", "section", "h1"]);
        assert_eq!(info.crumbs.last().unwrap().id, h1);
        for (crumb, tag) in info.crumbs.iter().zip(["html", "body", "div", "section", "h1"]) {
            assert_eq!(d.tag(crumb.id).as_deref(), Some(tag));
            assert!(d.is_attached(crumb.id));
        }
        // Y cada tramo se puede seleccionar: tiene su propia ruta, prefijo de la completa.
        let div = info.crumbs[2].id;
        assert_eq!(d.selection_info(div).unwrap().path, "html > body > div.hero");
    }

    #[test]
    fn lookup_helpers_and_attachment_tracking() {
        let mut d = Document::parse(SAMPLE);
        let mut h = History::default();
        let body = d.find_tag("body").unwrap();
        let div = id_of_tag(&d, "div");
        assert_eq!(d.child_at(body, 0), Some(div));
        assert_eq!(d.child_at(body, 9), None);
        assert!(d.is_attached(div));

        h.execute(&mut d, Box::new(DeleteNode::new(div))).unwrap();
        assert!(!d.is_attached(div));
        h.undo(&mut d).unwrap();
        assert!(d.is_attached(div));
        assert!(!d.is_attached(9999));
    }

    #[test]
    fn message_roundtrip() {
        let m: FromWebview = serde_json::from_str(r#"{"type":"Ready"}"#).unwrap();
        assert!(matches!(m, FromWebview::Ready));
        let m: FromWebview = serde_json::from_str(r#"{"type":"Select","id":7}"#).unwrap();
        assert!(matches!(m, FromWebview::Select { id: 7 }));
        let m: FromWebview = serde_json::from_str(r#"{"type":"SetAttr","id":3,"name":"class","value":null}"#).unwrap();
        assert!(matches!(m, FromWebview::SetAttr { id: 3, value: None, .. }));
        let m: FromWebview = serde_json::from_str(r#"{"type":"Undo"}"#).unwrap();
        assert!(matches!(m, FromWebview::Undo));
        let m: FromWebview = serde_json::from_str(r#"{"type":"SetViewport","width":375}"#).unwrap();
        assert!(matches!(m, FromWebview::SetViewport { width: Some(375) }));
        let m: FromWebview = serde_json::from_str(r#"{"type":"SetViewport","width":null}"#).unwrap();
        assert!(matches!(m, FromWebview::SetViewport { width: None }));
        let m: FromWebview = serde_json::from_str(r#"{"type":"SetTheme","theme":"dark"}"#).unwrap();
        assert!(matches!(m, FromWebview::SetTheme { ref theme } if theme == "dark"));
        let m: FromWebview =
            serde_json::from_str(r#"{"type":"SetDecl","style":4,"rule":1,"name":"color","value":null}"#).unwrap();
        assert!(matches!(m, FromWebview::SetDecl { style: 4, rule: 1, value: None, .. }));
        let m: FromWebview = serde_json::from_str(r#"{"type":"SetSource","html":"<p>x</p>"}"#).unwrap();
        assert!(matches!(m, FromWebview::SetSource { ref html } if html == "<p>x</p>"));
        let m: FromWebview = serde_json::from_str(r#"{"type":"SetCodeView","open":false}"#).unwrap();
        assert!(matches!(m, FromWebview::SetCodeView { open: false }));
        let m: FromWebview = serde_json::from_str(r#"{"type":"ActivateFile","path":null}"#).unwrap();
        assert!(matches!(m, FromWebview::ActivateFile { path: None }));
        let m: FromWebview = serde_json::from_str(r#"{"type":"OpenCss","path":"css/a.css"}"#).unwrap();
        assert!(matches!(m, FromWebview::OpenCss { ref path } if path == "css/a.css"));
        let m: FromWebview =
            serde_json::from_str(r#"{"type":"SetFileText","path":"a.css","text":"p{}"}"#).unwrap();
        assert!(matches!(m, FromWebview::SetFileText { ref text, .. } if text == "p{}"));
        let m: FromWebview = serde_json::from_str(
            r#"{"type":"SetFileDecl","path":"a.css","rule":2,"name":"color","value":"red"}"#,
        )
        .unwrap();
        assert!(matches!(m, FromWebview::SetFileDecl { rule: 2, value: Some(ref v), .. } if v == "red"));
        let m: FromWebview = serde_json::from_str(r#"{"type":"InsertImage","path":"img/a.png"}"#).unwrap();
        assert!(matches!(m, FromWebview::InsertImage { ref path } if path == "img/a.png"));
        let m: FromWebview = serde_json::from_str(r#"{"type":"NewFile","path":"css/n.css"}"#).unwrap();
        assert!(matches!(m, FromWebview::NewFile { ref path } if path == "css/n.css"));
        let m: FromWebview = serde_json::from_str(r#"{"type":"IndentHtml"}"#).unwrap();
        assert!(matches!(m, FromWebview::IndentHtml));
        let m: FromWebview = serde_json::from_str(r#"{"type":"MinifyHtml"}"#).unwrap();
        assert!(matches!(m, FromWebview::MinifyHtml));
        let m: FromWebview = serde_json::from_str(r#"{"type":"DeletePath","path":"img"}"#).unwrap();
        assert!(matches!(m, FromWebview::DeletePath { ref path } if path == "img"));
        let m: FromWebview =
            serde_json::from_str(r#"{"type":"RenamePath","path":"css/a.css","new_name":"b.css"}"#).unwrap();
        assert!(
            matches!(m, FromWebview::RenamePath { ref path, ref new_name } if path == "css/a.css" && new_name == "b.css")
        );
        let m: FromWebview = serde_json::from_str(r#"{"type":"RefreshFiles"}"#).unwrap();
        assert!(matches!(m, FromWebview::RefreshFiles));
        let m: FromWebview = serde_json::from_str(r#"{"type":"SearchSite"}"#).unwrap();
        assert!(matches!(m, FromWebview::SearchSite));
        let m: FromWebview =
            serde_json::from_str(r#"{"type":"GoTo","path":"a.css","line":3,"col":4,"len":5}"#).unwrap();
        assert!(matches!(m, FromWebview::GoTo { ref path, line: 3, col: 4, len: 5 } if path == "a.css"));
        let m: FromWebview = serde_json::from_str(r#"{"type":"SaveSnippet","name":"Mi tarjeta"}"#).unwrap();
        assert!(matches!(m, FromWebview::SaveSnippet { ref name } if name == "Mi tarjeta"));
        let m: FromWebview = serde_json::from_str(r#"{"type":"InsertSnippet","index":2}"#).unwrap();
        assert!(matches!(m, FromWebview::InsertSnippet { index: 2 }));
        let m: FromWebview = serde_json::from_str(r#"{"type":"DeleteSnippet","index":7}"#).unwrap();
        assert!(matches!(m, FromWebview::DeleteSnippet { index: 7 }));
        let m: FromWebview = serde_json::from_str(r#"{"type":"SetAutoReview","on":false}"#).unwrap();
        assert!(matches!(m, FromWebview::SetAutoReview { on: false }));
        let m: FromWebview = serde_json::from_str(r#"{"type":"FixIssue","index":4}"#).unwrap();
        assert!(matches!(m, FromWebview::FixIssue { index: 4 }));
        let m: FromWebview = serde_json::from_str(
            r#"{"type":"ReplaceInSite","count":3,"files":[{"path":"a.css","from_len":4,"text":"b{}"}]}"#,
        )
        .unwrap();
        assert!(matches!(m, FromWebview::ReplaceInSite { count: 3, ref files }
            if files.len() == 1 && files[0].from_len == 4 && files[0].text == "b{}"));
        let m: FromWebview = serde_json::from_str(r#"{"type":"Matches","id":9,"rules":[true,false]}"#).unwrap();
        assert!(matches!(m, FromWebview::Matches { id: 9, ref rules } if rules == &[true, false]));
    }
}
