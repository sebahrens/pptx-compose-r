use pptx_compose_core::{
    error::{Error, ErrorCode, ErrorLocation, Result},
    opc::package::Package,
    pptx::{ids::ElementKind, text::read_text_body},
    xml::{
        document::{QualifiedName, XmlAttribute, XmlDocument, XmlElement, XmlNode},
        parser::parse_document,
        writer::{WriteMode, WriteOptions, write_document},
    },
};
use pptx_compose_json::schemas::OperationTarget;
use serde_json::json;

use crate::{
    operations::ResolvedElement,
    patch::{FormatPolicy, OverflowPolicy, PatchEffects, ReplaceTextMode, ReplaceTextOperation},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplaceText {
    pub operation_id: String,
    pub element_id: String,
    pub text: String,
    pub current_text_match: Option<String>,
    pub mode: ReplaceTextMode,
    pub format_policy: FormatPolicy,
    pub overflow_policy: OverflowPolicy,
}

impl From<&ReplaceTextOperation> for ReplaceText {
    fn from(operation: &ReplaceTextOperation) -> Self {
        Self {
            operation_id: operation.operation_id.clone(),
            element_id: operation.target_element_id().to_owned(),
            text: operation.text.clone(),
            current_text_match: operation.current_text_match.clone(),
            mode: operation.mode.unwrap_or(ReplaceTextMode::WholeElement),
            format_policy: operation
                .format_policy
                .unwrap_or(FormatPolicy::PreserveExistingRuns),
            overflow_policy: operation.overflow_policy.unwrap_or(OverflowPolicy::Allow),
        }
    }
}

impl ReplaceText {
    pub fn validate(&self, package: &Package, target: &ResolvedElement) -> Result<()> {
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
        let element = target_element(&document, target)
            .ok_or_else(|| self.not_found("Target element path no longer resolves."))?;
        let tx_body = child_element(element, "txBody").ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                "Target element does not contain a text body.",
            )
            .with_location(self.location(Some(target)))
        })?;
        self.validate_match(target, tx_body)
    }

    pub fn apply(&self, package: &mut Package, target: &ResolvedElement) -> Result<PatchEffects> {
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

        let formatting_simplified = rewrite_text_body(&mut document, self, target)?;
        *part.bytes_mut() = write_document(
            &document,
            &WriteOptions {
                mode: WriteMode::Preserve,
            },
        )?;
        package.mark_dirty(part_name.clone());

        let mut warnings = vec![json!({ "newline_mapping": "paragraph" })];
        if formatting_simplified {
            warnings.push(json!({
                "code": "formatting_simplified",
                "message": "Existing rich text structure could not be preserved exactly."
            }));
        }

        Ok(PatchEffects {
            changed_parts: vec![part_name.zip_entry_name().to_owned()],
            target: Some(OperationTarget {
                slide_id: target.slide_id.clone(),
                element_id: target.element_id.clone(),
                part: part_name.zip_entry_name().to_owned(),
            }),
            created_element_ids: Vec::new(),
            warnings,
        })
    }

    fn validate_target(&self, target: &ResolvedElement) -> Result<()> {
        if target.element_id != self.element_id {
            return Err(Error::new(
                ErrorCode::SelectorGuardFailed,
                "Resolved element does not match replace_text element_id.",
            )
            .with_location(self.location(Some(target))));
        }
        if !matches!(target.kind, ElementKind::TextBox | ElementKind::Shape) {
            return Err(Error::new(
                ErrorCode::UnsupportedEdit,
                "Target element is not text-capable.",
            )
            .with_location(self.location(Some(target))));
        }
        Ok(())
    }

    fn validate_match(&self, target: &ResolvedElement, tx_body: &XmlElement) -> Result<()> {
        let Some(expected) = &self.current_text_match else {
            return Ok(());
        };
        let current = read_text_body(tx_body).plain;
        if current != *expected {
            return Err(Error::new(
                ErrorCode::SelectorGuardFailed,
                "replace_text match guard did not match current element text.",
            )
            .with_location(self.location(Some(target))));
        }
        Ok(())
    }

    fn not_found(&self, message: impl Into<String>) -> Error {
        Error::new(ErrorCode::SelectorNotFound, message).with_location(self.location(None))
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
            operation: Some("replace_text".to_owned()),
            ..ErrorLocation::default()
        }
    }
}

fn rewrite_text_body(
    document: &mut XmlDocument,
    operation: &ReplaceText,
    target: &ResolvedElement,
) -> Result<bool> {
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
    let tx_body_index = element
        .children
        .iter()
        .position(|node| {
            node.as_element()
                .is_some_and(|child| child.name.local_name == "txBody")
        })
        .ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                "Target element does not contain a text body.",
            )
            .with_location(operation.location(Some(target)))
        })?;

    let tx_body = element.children[tx_body_index]
        .as_element()
        .ok_or_else(|| {
            Error::new(
                ErrorCode::InternalError,
                "Text body node is not an element.",
            )
        })?;
    operation.validate_match(target, tx_body)?;
    let replacement = replacement_text_body(tx_body, operation);
    let formatting_simplified = should_warn_formatting_simplified(tx_body, operation);
    element.children[tx_body_index] = XmlNode::Element(replacement);
    Ok(formatting_simplified)
}

fn replacement_text_body(existing: &XmlElement, operation: &ReplaceText) -> XmlElement {
    let mut children = existing
        .children
        .iter()
        .filter(|node| {
            node.as_element().is_some_and(|child| {
                matches!(child.name.local_name.as_str(), "bodyPr" | "lstStyle")
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    if !children.iter().any(|node| {
        node.as_element()
            .is_some_and(|child| child.name.local_name == "bodyPr")
    }) {
        children.push(node(element("a:bodyPr", &[], Vec::new())));
    }
    if !children.iter().any(|node| {
        node.as_element()
            .is_some_and(|child| child.name.local_name == "lstStyle")
    }) {
        children.push(node(element("a:lstStyle", &[], Vec::new())));
    }

    let run_properties = match operation.format_policy {
        FormatPolicy::PreserveExistingRuns | FormatPolicy::PreserveFirstRun => {
            first_run_properties(existing).cloned()
        }
        FormatPolicy::SingleRunDefaultStyle => None,
    };
    children.extend(
        operation
            .text
            .split('\n')
            .map(|paragraph_text| node(paragraph(paragraph_text, run_properties.as_ref()))),
    );

    XmlElement {
        name: existing.name.clone(),
        attributes: existing.attributes.clone(),
        namespaces: existing.namespaces.clone(),
        children,
    }
}

fn should_warn_formatting_simplified(existing: &XmlElement, operation: &ReplaceText) -> bool {
    if operation.format_policy == FormatPolicy::SingleRunDefaultStyle {
        return first_run_properties(existing).is_some() || run_count(existing) > 1;
    }
    run_count(existing) > 1 || contains_rich_text_construct(existing)
}

fn paragraph(text: &str, run_properties: Option<&XmlElement>) -> XmlElement {
    element("a:p", &[], vec![node(run(text, run_properties))])
}

fn run(text: &str, run_properties: Option<&XmlElement>) -> XmlElement {
    let mut children = Vec::new();
    if let Some(run_properties) = run_properties {
        children.push(node(run_properties.clone()));
    }
    children.push(node(element(
        "a:t",
        &[],
        vec![XmlNode::Text(text.to_owned())],
    )));
    element("a:r", &[], children)
}

fn first_run_properties(element: &XmlElement) -> Option<&XmlElement> {
    for child in element.children.iter().filter_map(XmlNode::as_element) {
        if child.name.local_name == "r" {
            if let Some(run_properties) = child_element(child, "rPr") {
                return Some(run_properties);
            }
        } else if let Some(run_properties) = first_run_properties(child) {
            return Some(run_properties);
        }
    }
    None
}

fn run_count(element: &XmlElement) -> usize {
    element
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .map(|child| {
            usize::from(child.name.local_name == "r")
                + if child.name.local_name == "r" {
                    0
                } else {
                    run_count(child)
                }
        })
        .sum()
}

fn contains_rich_text_construct(element: &XmlElement) -> bool {
    element
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .any(|child| {
            matches!(
                child.name.local_name.as_str(),
                "fld" | "hlinkClick" | "hlinkMouseOver" | "br"
            ) || contains_rich_text_construct(child)
        })
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
            .nth(index)?;
    }
    Some(current)
}

fn child_element<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a XmlElement> {
    element
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .find(|child| child.name.local_name == local_name)
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

fn node(element: XmlElement) -> XmlNode {
    XmlNode::Element(element)
}

fn element(raw_name: &str, attrs: &[(&str, &str)], children: Vec<XmlNode>) -> XmlElement {
    XmlElement {
        name: QualifiedName::from_raw(raw_name),
        attributes: attrs
            .iter()
            .map(|(name, value)| XmlAttribute {
                name: QualifiedName::from_raw(*name),
                value: (*value).to_owned(),
                namespace_declaration: false,
            })
            .collect(),
        namespaces: Default::default(),
        children,
    }
}

#[cfg(test)]
#[test]
fn replaces_and_maps_newlines() {
    use pptx_compose_core::opc::part_name::PartName;

    let slide_part = PartName::from_zip_entry("ppt/slides/slide1.xml").expect("valid slide part");
    let mut package = Package::new();
    package
        .insert_zip_entry(
            "ppt/slides/slide1.xml",
            br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvSpPr txBox="1"/></p:nvSpPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US" dirty="0" b="1"/><a:t>old</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#.to_vec(),
        )
        .expect("slide inserted");
    let target = ResolvedElement {
        slide_id: "slide-1".to_owned(),
        element_id: "slide-1:shape-3".to_owned(),
        kind: ElementKind::TextBox,
        part: slide_part.clone(),
        sp_tree_path: vec![3],
        group_path: Vec::new(),
        cnvpr_id: None,
        text_hash: None,
        fingerprint: "fp".to_owned(),
    };
    let operation = ReplaceText {
        operation_id: "op-1".to_owned(),
        element_id: target.element_id.clone(),
        text: "a\nb".to_owned(),
        current_text_match: Some("old".to_owned()),
        mode: ReplaceTextMode::WholeElement,
        format_policy: FormatPolicy::PreserveFirstRun,
        overflow_policy: OverflowPolicy::Allow,
    };

    let effects = operation
        .apply(&mut package, &target)
        .expect("text is replaced");

    assert_eq!(effects.changed_parts, vec!["ppt/slides/slide1.xml"]);
    assert!(package.dirty_parts().contains(&slide_part));
    assert!(
        effects
            .warnings
            .contains(&json!({ "newline_mapping": "paragraph" }))
    );

    let slide_xml = String::from_utf8(
        package
            .parts()
            .get(&slide_part)
            .expect("slide still exists")
            .bytes()
            .to_vec(),
    )
    .expect("slide XML is UTF-8");
    assert_eq!(slide_xml.matches("<a:p>").count(), 2);
    assert!(slide_xml.contains(r#"<a:t>a</a:t>"#));
    assert!(slide_xml.contains(r#"<a:t>b</a:t>"#));
    assert!(slide_xml.contains(r#"<a:rPr lang="en-US" dirty="0" b="1"/>"#));

    let guarded = ReplaceText {
        current_text_match: Some("old".to_owned()),
        text: "again".to_owned(),
        ..operation
    };
    let error = guarded
        .validate(&package, &target)
        .expect_err("stale match guard fails");
    assert_eq!(error.code(), ErrorCode::SelectorGuardFailed);
}
