use pptx_compose_core::{
    error::{Error, ErrorCode, ErrorLocation, Result},
    opc::package::Package,
    xml::{
        document::{XmlAttribute, XmlDocument, XmlElement, XmlNode},
        parser::parse_document,
        writer::{WriteMode, WriteOptions, write_document},
    },
};
use pptx_compose_json::schemas::OperationTarget;

use crate::{
    operations::{ResolvedElement, is_real_shape_tree_child},
    patch::{PatchEffects, SetAltTextOperation},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetAltText {
    pub operation_id: String,
    pub element_id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub alt_text: Option<String>,
}

impl From<&SetAltTextOperation> for SetAltText {
    fn from(operation: &SetAltTextOperation) -> Self {
        Self {
            operation_id: operation.operation_id.clone(),
            element_id: operation.target_element_id().to_owned(),
            title: operation.title.clone(),
            description: operation.description.clone(),
            alt_text: operation.alt_text.clone(),
        }
    }
}

impl SetAltText {
    pub fn validate(&self, package: &Package, target: &ResolvedElement) -> Result<()> {
        self.validate_fields()?;
        self.validate_target(target)?;

        let part_name = target.part.clone();
        let part = package.parts().get(&part_name).ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                format!("Target slide part {part_name} was not found."),
            )
            .with_location(self.location(Some(target)))
        })?;
        let document = parse_document(part.bytes()).map_err(|source| {
            Error::with_source(
                source.code(),
                format!("Could not parse target slide part {part_name}."),
                source,
            )
            .with_location(self.location(Some(target)))
        })?;
        let element = target_element(&document, target).ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                "Target element path no longer resolves in the slide shape tree.",
            )
            .with_location(self.location(Some(target)))
        })?;
        cnv_pr(element).ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                "Target element does not contain DrawingML non-visual properties.",
            )
            .with_location(self.location(Some(target)))
        })?;

        Ok(())
    }

    pub fn apply(&self, package: &mut Package, target: &ResolvedElement) -> Result<PatchEffects> {
        self.validate_fields()?;
        self.validate_target(target)?;

        let part_name = target.part.clone();
        let part = package.parts_mut().get_mut(&part_name).ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                format!("Target slide part {part_name} was not found."),
            )
            .with_location(self.location(Some(target)))
        })?;

        let mut document = parse_document(part.bytes()).map_err(|source| {
            Error::with_source(
                source.code(),
                format!("Could not parse target slide part {part_name}."),
                source,
            )
            .with_location(self.location(Some(target)))
        })?;

        rewrite_cnv_pr(&mut document, self, target)?;
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

    fn validate_fields(&self) -> Result<()> {
        if self.title.is_none() && self.description.is_none() && self.alt_text.is_none() {
            return Err(Error::new(
                ErrorCode::InvalidInput,
                "set_alt_text requires at least one of title, description, or alt_text.",
            )
            .with_location(self.location(None)));
        }
        Ok(())
    }

    fn validate_target(&self, target: &ResolvedElement) -> Result<()> {
        if target.element_id != self.element_id {
            return Err(Error::new(
                ErrorCode::SelectorGuardFailed,
                "Resolved element does not match set_alt_text element_id.",
            )
            .with_location(self.location(Some(target))));
        }
        Ok(())
    }

    fn location(&self, target: Option<&ResolvedElement>) -> ErrorLocation {
        ErrorLocation {
            part: target.map(|target| target.part.zip_entry_name().to_owned()),
            slide_id: target.map(|target| target.slide_id.clone()),
            element_id: Some(
                target
                    .map(|target| target.element_id.clone())
                    .unwrap_or_else(|| self.element_id.clone()),
            ),
            operation_id: Some(self.operation_id.clone()),
            operation: Some("set_alt_text".to_owned()),
            ..ErrorLocation::default()
        }
    }
}

fn rewrite_cnv_pr(
    document: &mut XmlDocument,
    operation: &SetAltText,
    target: &ResolvedElement,
) -> Result<()> {
    let root = root_element_mut(document).ok_or_else(|| {
        Error::malformed_xml("Slide XML does not contain a root element.")
            .with_location(operation.location(Some(target)))
    })?;
    let sp_tree = first_descendant_mut(root, "spTree").ok_or_else(|| {
        Error::new(
            ErrorCode::SelectorNotFound,
            "Target slide does not contain a shape tree.",
        )
        .with_location(operation.location(Some(target)))
    })?;
    let element = element_at_path_mut(sp_tree, &target.sp_tree_path).ok_or_else(|| {
        Error::new(
            ErrorCode::SelectorNotFound,
            "Target element path no longer resolves in the slide shape tree.",
        )
        .with_location(operation.location(Some(target)))
    })?;
    let cnv_pr = cnv_pr_mut(element).ok_or_else(|| {
        Error::new(
            ErrorCode::UnsupportedEdit,
            "Target element does not contain DrawingML non-visual properties.",
        )
        .with_location(operation.location(Some(target)))
    })?;

    if let Some(description) = operation
        .description
        .as_ref()
        .or(operation.alt_text.as_ref())
    {
        set_attribute(cnv_pr, "descr", description);
    }
    if let Some(title) = &operation.title {
        set_attribute(cnv_pr, "title", title);
    }

    Ok(())
}

fn target_element<'a>(
    document: &'a XmlDocument,
    target: &ResolvedElement,
) -> Option<&'a XmlElement> {
    let root = document.root_element()?;
    let sp_tree = first_descendant(root, "spTree")?;
    element_at_path(sp_tree, &target.sp_tree_path)
}

fn root_element_mut(document: &mut XmlDocument) -> Option<&mut XmlElement> {
    document.nodes.iter_mut().find_map(node_element_mut)
}

fn first_descendant<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a XmlElement> {
    for child in element.children.iter().filter_map(XmlNode::as_element) {
        if child.name.local_name == local_name {
            return Some(child);
        }
        if let Some(descendant) = first_descendant(child, local_name) {
            return Some(descendant);
        }
    }
    None
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

fn element_at_path<'a>(sp_tree: &'a XmlElement, path: &[u32]) -> Option<&'a XmlElement> {
    let mut current = sp_tree;
    for component in path {
        let index = usize::try_from(component.checked_sub(1)?).ok()?;
        current = current
            .children
            .iter()
            .filter_map(XmlNode::as_element)
            .filter(|element| is_real_shape_tree_child(element))
            .nth(index)?;
    }
    Some(current)
}

fn element_at_path_mut<'a>(
    sp_tree: &'a mut XmlElement,
    path: &[u32],
) -> Option<&'a mut XmlElement> {
    let mut current = sp_tree;
    for component in path {
        let index = usize::try_from(component.checked_sub(1)?).ok()?;
        current = current
            .children
            .iter_mut()
            .filter_map(node_element_mut)
            .filter(|element| is_real_shape_tree_child(element))
            .nth(index)?;
    }
    Some(current)
}

fn cnv_pr(element: &XmlElement) -> Option<&XmlElement> {
    first_descendant(element, "cNvPr")
}

fn cnv_pr_mut(element: &mut XmlElement) -> Option<&mut XmlElement> {
    first_descendant_mut(element, "cNvPr")
}

fn set_attribute(element: &mut XmlElement, name: &str, value: &str) {
    if let Some(attribute) = element
        .attributes
        .iter_mut()
        .find(|attribute| attribute.name.local_name == name)
    {
        attribute.value = value.to_owned();
        return;
    }

    element.attributes.push(XmlAttribute {
        name: pptx_compose_core::xml::document::QualifiedName::from_raw(name),
        value: value.to_owned(),
        namespace_declaration: false,
    });
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

#[cfg(test)]
#[test]
fn sets_descr_title() {
    use pptx_compose_core::{
        opc::part_name::PartName, pptx::ids::ElementKind, xml::parser::parse_document,
    };

    let slide_part = PartName::from_zip_entry("ppt/slides/slide1.xml").expect("valid slide part");
    let mut package = Package::new();
    package
        .insert_zip_entry(
            "ppt/slides/slide1.xml",
            br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="9" name="Existing" hidden="0"/></p:nvSpPr><p:spPr/></p:sp></p:spTree></p:cSld></p:sld>"#.to_vec(),
        )
        .expect("slide inserted");
    let target = ResolvedElement {
        slide_id: "slide-1".to_owned(),
        element_id: "slide-1:shape-1".to_owned(),
        kind: ElementKind::Shape,
        part: slide_part.clone(),
        sp_tree_path: vec![1],
        group_path: Vec::new(),
        cnvpr_id: Some(9),
        text_hash: None,
        fingerprint: "fp".to_owned(),
    };
    let operation = SetAltText {
        operation_id: "op-1".to_owned(),
        element_id: target.element_id.clone(),
        title: Some("Accessible title".to_owned()),
        description: Some("Readable description".to_owned()),
        alt_text: None,
    };

    let effects = operation
        .apply(&mut package, &target)
        .expect("alt text is applied");

    assert_eq!(effects.changed_parts, vec!["ppt/slides/slide1.xml"]);
    assert!(package.dirty_parts().contains(&slide_part));
    let document = parse_document(
        package
            .parts()
            .get(&slide_part)
            .expect("slide still exists")
            .bytes(),
    )
    .expect("updated slide parses");
    let element = target_element(&document, &target).expect("target still resolves");
    let cnv_pr = cnv_pr(element).expect("cNvPr still exists");
    assert_eq!(attr(cnv_pr, "id"), Some("9"));
    assert_eq!(attr(cnv_pr, "name"), Some("Existing"));
    assert_eq!(attr(cnv_pr, "hidden"), Some("0"));
    assert_eq!(attr(cnv_pr, "descr"), Some("Readable description"));
    assert_eq!(attr(cnv_pr, "title"), Some("Accessible title"));

    let empty = SetAltText {
        operation_id: "op-empty".to_owned(),
        element_id: target.element_id.clone(),
        title: None,
        description: None,
        alt_text: None,
    };
    let error = empty
        .validate(&package, &target)
        .expect_err("empty operation is invalid");
    assert_eq!(error.code(), ErrorCode::InvalidInput);
}

#[cfg(test)]
fn attr<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local_name == local_name)
        .map(|attribute| attribute.value.as_str())
}
