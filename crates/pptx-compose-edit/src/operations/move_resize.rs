use pptx_compose_core::{
    error::{Error, ErrorCode, ErrorLocation, Result},
    opc::package::Package,
    pptx::ids::ElementKind,
    xml::{
        document::{QualifiedName, XmlAttribute, XmlDocument, XmlElement, XmlNode},
        parser::parse_document,
        writer::{WriteMode, WriteOptions, write_document},
    },
};
use pptx_compose_json::schemas::OperationTarget;

use crate::{
    operations::{ResolvedElement, bounds::validate_bounds},
    patch::{Bounds, MoveResizeElementOperation, PatchEffects},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoveResize {
    pub element_id: String,
    pub bounds: Bounds,
}

impl From<&MoveResizeElementOperation> for MoveResize {
    fn from(operation: &MoveResizeElementOperation) -> Self {
        Self {
            element_id: operation.element_id.clone(),
            bounds: operation.bounds.clone(),
        }
    }
}

impl MoveResize {
    pub fn validate(&self) -> Result<()> {
        validate_bounds(&self.bounds).map_err(|error| {
            error.with_location(ErrorLocation {
                element_id: Some(self.element_id.clone()),
                ..ErrorLocation::default()
            })
        })
    }

    pub fn apply(&self, package: &mut Package, target: &ResolvedElement) -> Result<PatchEffects> {
        self.validate()?;
        ensure_movable(target)?;

        let part_name = target.part.clone();
        let part = package.parts_mut().get_mut(&part_name).ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                format!("Target slide part {part_name} was not found."),
            )
            .with_location(location(target))
        })?;

        let mut document = parse_document(part.bytes()).map_err(|source| {
            Error::with_source(
                source.code(),
                format!("Could not parse target slide part {part_name}."),
                source,
            )
            .with_location(location(target))
        })?;

        rewrite_bounds(&mut document, target, &self.bounds)?;

        *part.bytes_mut() = write_document(
            &document,
            &WriteOptions {
                mode: WriteMode::Preserve,
            },
        )?;
        package.mark_dirty(part_name.clone());

        Ok(PatchEffects {
            changed_parts: vec![part_name.zip_entry_name().to_owned()],
            target: Some(OperationTarget {
                slide_id: target.slide_id.clone(),
                element_id: target.element_id.clone(),
                part: part_name.zip_entry_name().to_owned(),
            }),
            created_element_ids: Vec::new(),
            warnings: Vec::new(),
        })
    }
}

fn ensure_movable(target: &ResolvedElement) -> Result<()> {
    match target.kind {
        ElementKind::TextBox
        | ElementKind::Shape
        | ElementKind::Picture
        | ElementKind::GraphicFrame => Ok(()),
        ElementKind::Group | ElementKind::Connector | ElementKind::Other => Err(Error::new(
            ErrorCode::UnsupportedEdit,
            "Target element does not have movable DrawingML bounds.",
        )
        .with_location(location(target))),
    }
}

fn rewrite_bounds(
    document: &mut XmlDocument,
    target: &ResolvedElement,
    bounds: &Bounds,
) -> Result<()> {
    let root = root_element_mut(document).ok_or_else(|| {
        Error::malformed_xml("Slide XML does not contain a root element.")
            .with_location(location(target))
    })?;
    let sp_tree = first_descendant_mut(root, "spTree").ok_or_else(|| {
        Error::new(
            ErrorCode::SelectorNotFound,
            "Target slide does not contain a shape tree.",
        )
        .with_location(location(target))
    })?;
    let element = element_at_path_mut(sp_tree, &target.sp_tree_path).ok_or_else(|| {
        Error::new(
            ErrorCode::SelectorNotFound,
            "Target element path no longer resolves in the slide shape tree.",
        )
        .with_location(location(target))
    })?;
    let xfrm = xfrm_for_target_mut(element, target)?;
    set_child_empty_element(xfrm, "off", &[("x", bounds.x), ("y", bounds.y)]);
    set_child_empty_element(xfrm, "ext", &[("cx", bounds.cx), ("cy", bounds.cy)]);
    Ok(())
}

fn xfrm_for_target_mut<'a>(
    element: &'a mut XmlElement,
    target: &ResolvedElement,
) -> Result<&'a mut XmlElement> {
    if target.kind == ElementKind::GraphicFrame {
        ensure_child_element(element, "xfrm", "p:xfrm");
        return child_element_mut(element, "xfrm").ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                "Graphic frame bounds could not be created.",
            )
            .with_location(location(target))
        });
    }

    ensure_child_element(element, "spPr", "p:spPr");
    let sp_pr = child_element_mut(element, "spPr").ok_or_else(|| {
        Error::new(
            ErrorCode::UnsupportedEdit,
            "Target element shape properties could not be created.",
        )
        .with_location(location(target))
    })?;
    ensure_child_element(sp_pr, "xfrm", "a:xfrm");
    child_element_mut(sp_pr, "xfrm").ok_or_else(|| {
        Error::new(
            ErrorCode::UnsupportedEdit,
            "Target element transform could not be created.",
        )
        .with_location(location(target))
    })
}

fn root_element_mut(document: &mut XmlDocument) -> Option<&mut XmlElement> {
    document.nodes.iter_mut().find_map(node_element_mut)
}

fn first_descendant_mut<'a>(
    element: &'a mut XmlElement,
    local_name: &str,
) -> Option<&'a mut XmlElement> {
    for child in &mut element.children {
        let Some(child_element) = node_element_mut(child) else {
            continue;
        };
        if child_element.name.local_name == local_name {
            return Some(child_element);
        }
        if let Some(descendant) = first_descendant_mut(child_element, local_name) {
            return Some(descendant);
        }
    }
    None
}

fn element_at_path_mut<'a>(
    sp_tree: &'a mut XmlElement,
    path: &[u32],
) -> Option<&'a mut XmlElement> {
    let mut current = sp_tree;
    for component in path {
        let wanted = usize::try_from(component.checked_sub(1)?).ok()?;
        current = current
            .children
            .iter_mut()
            .filter_map(node_element_mut)
            .nth(wanted)?;
    }
    Some(current)
}

fn child_element_mut<'a>(
    element: &'a mut XmlElement,
    local_name: &str,
) -> Option<&'a mut XmlElement> {
    element
        .children
        .iter_mut()
        .filter_map(node_element_mut)
        .find(|child| child.name.local_name == local_name)
}

fn ensure_child_element(element: &mut XmlElement, local_name: &str, raw_name: &str) {
    if element
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .any(|child| child.name.local_name == local_name)
    {
        return;
    }
    element
        .children
        .push(XmlNode::Element(empty_element(raw_name)));
}

fn set_child_empty_element(parent: &mut XmlElement, local_name: &str, attrs: &[(&str, i64)]) {
    if let Some(child) = child_element_mut(parent, local_name) {
        child.attributes.retain(|attribute| {
            !attrs
                .iter()
                .any(|(name, _)| attribute.name.local_name == *name)
        });
        child.children.clear();
        for (name, value) in attrs {
            child.attributes.push(attribute(name, *value));
        }
        return;
    }

    let mut child = empty_element(&format!("a:{local_name}"));
    child.attributes = attrs
        .iter()
        .map(|(name, value)| attribute(name, *value))
        .collect();
    parent.children.push(XmlNode::Element(child));
}

fn node_element_mut(node: &mut XmlNode) -> Option<&mut XmlElement> {
    match node {
        XmlNode::Element(element) => Some(element),
        XmlNode::Text(_)
        | XmlNode::CData(_)
        | XmlNode::Comment(_)
        | XmlNode::ProcessingInstruction(_)
        | XmlNode::DocType(_)
        | XmlNode::GeneralRef(_) => None,
    }
}

fn empty_element(raw_name: &str) -> XmlElement {
    XmlElement {
        name: QualifiedName::from_raw(raw_name),
        attributes: Vec::new(),
        namespaces: Default::default(),
        children: Vec::new(),
    }
}

fn attribute(name: &str, value: i64) -> XmlAttribute {
    XmlAttribute {
        name: QualifiedName::from_raw(name),
        value: value.to_string(),
        namespace_declaration: false,
    }
}

fn location(target: &ResolvedElement) -> ErrorLocation {
    ErrorLocation {
        part: Some(target.part.zip_entry_name().to_owned()),
        slide_id: Some(target.slide_id.clone()),
        element_id: Some(target.element_id.clone()),
        operation: Some("move_resize_element".to_owned()),
        ..ErrorLocation::default()
    }
}

#[cfg(test)]
#[test]
fn rejects_bounds_above_emu_max_without_writing() {
    let slide_part =
        pptx_compose_core::opc::part_name::PartName::from_zip_entry("ppt/slides/slide1.xml")
            .expect("valid slide part");
    let slide_bytes =
        br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:spPr><a:xfrm><a:off x="1" y="2"/><a:ext cx="3" cy="4"/></a:xfrm></p:spPr></p:sp></p:spTree></p:cSld></p:sld>"#.to_vec();
    let mut package = Package::new();
    package
        .insert_zip_entry("ppt/slides/slide1.xml", slide_bytes.clone())
        .expect("slide inserted");
    let target = ResolvedElement {
        slide_id: "slide-1".to_owned(),
        element_id: "slide-1:shape-3".to_owned(),
        kind: ElementKind::Shape,
        part: slide_part.clone(),
        sp_tree_path: vec![3],
        group_path: Vec::new(),
        cnvpr_id: None,
        text_hash: None,
        fingerprint: "fp".to_owned(),
    };
    let operation = MoveResize {
        element_id: target.element_id.clone(),
        bounds: Bounds {
            x: 0,
            y: 0,
            cx: crate::operations::bounds::MAX_EMU_COORDINATE + 1,
            cy: 1,
        },
    };

    let error = operation
        .apply(&mut package, &target)
        .expect_err("out-of-range width is invalid");

    assert_eq!(error.code(), ErrorCode::InvalidBounds);
    assert!(!package.dirty_parts().contains(&slide_part));
    assert_eq!(
        package
            .parts()
            .get(&slide_part)
            .expect("slide still exists")
            .bytes(),
        slide_bytes.as_slice()
    );
}

#[cfg(test)]
#[test]
fn preserves_rot_flip() {
    let slide_part =
        pptx_compose_core::opc::part_name::PartName::from_zip_entry("ppt/slides/slide1.xml")
            .expect("valid slide part");
    let mut package = Package::new();
    package
        .insert_zip_entry(
            "ppt/slides/slide1.xml",
            br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr/><p:spPr><a:xfrm rot="5400000" flipH="1"><a:off x="1" y="2"/><a:ext cx="3" cy="4"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr></p:sp><p:cxnSp><p:spPr/></p:cxnSp></p:spTree></p:cSld></p:sld>"#.to_vec(),
        )
        .expect("slide inserted");

    let target = ResolvedElement {
        slide_id: "slide-1".to_owned(),
        element_id: "slide-1:shape-3".to_owned(),
        kind: ElementKind::Shape,
        part: slide_part.clone(),
        sp_tree_path: vec![3],
        group_path: Vec::new(),
        cnvpr_id: None,
        text_hash: None,
        fingerprint: "fp".to_owned(),
    };
    let operation = MoveResize {
        element_id: target.element_id.clone(),
        bounds: Bounds {
            x: 10,
            y: 20,
            cx: 300,
            cy: 400,
        },
    };

    let effects = operation
        .apply(&mut package, &target)
        .expect("move/resize applies");

    assert_eq!(effects.changed_parts, vec!["ppt/slides/slide1.xml"]);
    assert!(package.dirty_parts().contains(&slide_part));
    let slide_xml = String::from_utf8(
        package
            .parts()
            .get(&slide_part)
            .expect("slide still exists")
            .bytes()
            .to_vec(),
    )
    .expect("slide XML is UTF-8");
    assert!(slide_xml.contains(r#"rot="5400000""#));
    assert!(slide_xml.contains(r#"flipH="1""#));
    assert!(slide_xml.contains(r#"<a:off x="10" y="20"/>"#));
    assert!(slide_xml.contains(r#"<a:ext cx="300" cy="400"/>"#));
    assert!(slide_xml.contains(r#"<a:prstGeom prst="rect"/>"#));

    let non_bounded = ResolvedElement {
        element_id: "slide-1:cxn-4".to_owned(),
        kind: ElementKind::Connector,
        sp_tree_path: vec![4],
        ..target
    };
    let error = operation
        .apply(&mut package, &non_bounded)
        .expect_err("non-bounded target is unsupported");
    assert_eq!(error.code(), ErrorCode::UnsupportedEdit);
}
