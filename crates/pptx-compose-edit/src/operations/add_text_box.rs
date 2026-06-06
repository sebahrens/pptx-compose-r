use pptx_compose_core::{
    error::{Error, ErrorCode, ErrorLocation, Result},
    opc::package::Package,
    pptx::ids::{ElementKind, SpTreePath, agent_element_id},
    xml::{
        document::{QualifiedName, XmlAttribute, XmlDocument, XmlElement, XmlNode},
        parser::parse_document,
        writer::{WriteMode, WriteOptions, write_document},
    },
};
use pptx_compose_json::schemas::OperationTarget;

use crate::{
    operations::{ResolvedSlide, bounds::validate_bounds, ensure_slide_namespaces},
    patch::{AddTextBoxOperation, Bounds, InsertOptions, PatchEffects, TextAlign, TextBoxStyle},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddTextBox {
    pub operation_id: String,
    pub slide_id: String,
    pub text: String,
    pub bounds: Bounds,
    pub name: Option<String>,
    pub alt_text: Option<String>,
    pub style: Option<TextBoxStyle>,
    pub insert: Option<InsertOptions>,
}

impl From<&AddTextBoxOperation> for AddTextBox {
    fn from(operation: &AddTextBoxOperation) -> Self {
        Self {
            operation_id: operation.operation_id.clone(),
            slide_id: operation.target_slide_id().to_owned(),
            text: operation.text.clone(),
            bounds: operation.bounds.clone(),
            name: operation.name.clone(),
            alt_text: operation.alt_text.clone(),
            style: operation.style.clone(),
            insert: operation.insert.clone(),
        }
    }
}

impl AddTextBox {
    pub fn validate(&self) -> Result<()> {
        validate_bounds(&self.bounds).map_err(|error| error.with_location(self.location(None)))?;
        validate_style(self.style.as_ref())
            .map_err(|error| error.with_location(self.location(None)))
    }

    pub fn apply(&self, package: &mut Package, target: &ResolvedSlide) -> Result<PatchEffects> {
        self.validate()?;
        if target.slide_id != self.slide_id {
            return Err(Error::new(
                ErrorCode::SelectorGuardFailed,
                "Resolved slide does not match add_text_box slide_id.",
            )
            .with_location(self.location(None)));
        }

        let part_name = target.part.clone();
        let part = package.parts_mut().get_mut(&part_name).ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                format!("Target slide part {part_name} was not found."),
            )
            .with_location(self.location(None))
        })?;

        let mut document = parse_document(part.bytes()).map_err(|source| {
            Error::with_source(
                source.code(),
                format!("Could not parse target slide part {part_name}."),
                source,
            )
            .with_location(self.location(None))
        })?;

        let element_id = insert_text_box(&mut document, self)?;
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
                element_id: element_id.clone(),
                part: part_name.zip_entry_name().to_owned(),
            }),
            created_element_ids: vec![element_id],
            warnings: Vec::new(),
        })
    }

    fn location(&self, element_id: Option<String>) -> ErrorLocation {
        ErrorLocation {
            slide_id: Some(self.slide_id.clone()),
            element_id,
            operation_id: Some(self.operation_id.clone()),
            operation: Some("add_text_box".to_owned()),
            ..ErrorLocation::default()
        }
    }
}

fn validate_style(style: Option<&TextBoxStyle>) -> Result<()> {
    let Some(style) = style else {
        return Ok(());
    };

    if let Some(key) = style.extra.keys().next() {
        return Err(Error::new(
            ErrorCode::UnsupportedEdit,
            format!("add_text_box.style field {key} is not supported in V1."),
        ));
    }

    if let Some(color) = &style.color_hex {
        let valid = color.len() == 6 && color.bytes().all(|byte| byte.is_ascii_hexdigit());
        if !valid {
            return Err(Error::new(
                ErrorCode::InvalidInput,
                "add_text_box.style.color_hex must be RRGGBB.",
            ));
        }
    }

    Ok(())
}

fn insert_text_box(document: &mut XmlDocument, operation: &AddTextBox) -> Result<String> {
    let root = root_element_mut(document).ok_or_else(|| {
        Error::malformed_xml("Slide XML does not contain a root element.")
            .with_location(operation.location(None))
    })?;
    ensure_slide_namespaces(root);
    let sp_tree = first_descendant_mut(root, "spTree").ok_or_else(|| {
        Error::new(
            ErrorCode::SelectorNotFound,
            "Target slide does not contain a shape tree.",
        )
        .with_location(operation.location(None))
    })?;
    let id = max_cnvpr_id(sp_tree).unwrap_or(0) + 1;
    let name = operation
        .name
        .clone()
        .unwrap_or_else(|| format!("TextBox {id}"));
    let shape = text_box_shape(id, &name, operation);
    sp_tree.children.push(XmlNode::Element(shape));

    let element_count = sp_tree
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .count();
    let Ok(path_component) = u32::try_from(element_count) else {
        return Err(Error::new(
            ErrorCode::UnsupportedEdit,
            "Shape tree has too many elements to assign an agent element id.",
        )
        .with_location(operation.location(None)));
    };
    let path = SpTreePath {
        sp_tree_path: vec![path_component],
        group_path: Vec::new(),
    };
    Ok(agent_element_id(
        &operation.slide_id,
        ElementKind::TextBox,
        Some(id),
        &path,
    ))
}

fn text_box_shape(id: i64, name: &str, operation: &AddTextBox) -> XmlElement {
    element(
        "p:sp",
        &[],
        vec![
            node(element(
                "p:nvSpPr",
                &[],
                vec![
                    node(cnv_pr(id, name, operation.alt_text.as_deref())),
                    node(element("p:cNvSpPr", &[("txBox", "1")], Vec::new())),
                    node(element("p:nvPr", &[], Vec::new())),
                ],
            )),
            node(element(
                "p:spPr",
                &[],
                vec![
                    node(element(
                        "a:xfrm",
                        &[],
                        vec![
                            node(element(
                                "a:off",
                                &[
                                    ("x", &operation.bounds.x.to_string()),
                                    ("y", &operation.bounds.y.to_string()),
                                ],
                                Vec::new(),
                            )),
                            node(element(
                                "a:ext",
                                &[
                                    ("cx", &operation.bounds.cx.to_string()),
                                    ("cy", &operation.bounds.cy.to_string()),
                                ],
                                Vec::new(),
                            )),
                        ],
                    )),
                    node(element(
                        "a:prstGeom",
                        &[("prst", "rect")],
                        vec![node(element("a:avLst", &[], Vec::new()))],
                    )),
                ],
            )),
            node(text_body(&operation.text, operation.style.as_ref())),
        ],
    )
}

fn cnv_pr(id: i64, name: &str, alt_text: Option<&str>) -> XmlElement {
    let id_string = id.to_string();
    let mut attrs = vec![("id", id_string.as_str()), ("name", name)];
    if let Some(alt_text) = alt_text {
        attrs.push(("descr", alt_text));
    }
    element("p:cNvPr", &attrs, Vec::new())
}

fn text_body(text: &str, style: Option<&TextBoxStyle>) -> XmlElement {
    let paragraphs: Vec<_> = text
        .split('\n')
        .map(|paragraph_text| node(paragraph(paragraph_text, style)))
        .collect();
    element(
        "p:txBody",
        &[],
        vec![
            node(element(
                "a:bodyPr",
                &[("wrap", "square"), ("rtlCol", "0")],
                vec![node(element("a:spAutoFit", &[], Vec::new()))],
            )),
            node(element("a:lstStyle", &[], Vec::new())),
        ]
        .into_iter()
        .chain(paragraphs)
        .collect(),
    )
}

fn paragraph(text: &str, style: Option<&TextBoxStyle>) -> XmlElement {
    let mut children = Vec::new();
    if let Some(style) = style
        && let Some(align) = style.align
    {
        children.push(node(element(
            "a:pPr",
            &[("algn", align_value(align))],
            Vec::new(),
        )));
    }
    children.push(node(run(text, style)));
    element("a:p", &[], children)
}

fn run(text: &str, style: Option<&TextBoxStyle>) -> XmlElement {
    element(
        "a:r",
        &[],
        vec![
            node(run_properties(style)),
            node(element("a:t", &[], vec![XmlNode::Text(text.to_owned())])),
        ],
    )
}

fn run_properties(style: Option<&TextBoxStyle>) -> XmlElement {
    let mut attrs = vec![
        ("lang".to_owned(), "en-US".to_owned()),
        ("dirty".to_owned(), "0".to_owned()),
    ];
    if let Some(style) = style {
        if let Some(font_size) = style.font_size_pt {
            attrs.push(("sz".to_owned(), (font_size * 100).to_string()));
        }
        if let Some(bold) = style.bold {
            attrs.push(("b".to_owned(), bool_value(bold).to_owned()));
        }
        if let Some(italic) = style.italic {
            attrs.push(("i".to_owned(), bool_value(italic).to_owned()));
        }
    }

    let attr_refs = attrs
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let mut children = Vec::new();
    if let Some(style) = style {
        if let Some(color) = &style.color_hex {
            children.push(node(element(
                "a:solidFill",
                &[],
                vec![node(element("a:srgbClr", &[("val", color)], Vec::new()))],
            )));
        }
        if let Some(font_family) = &style.font_family {
            children.push(node(element(
                "a:latin",
                &[("typeface", font_family)],
                Vec::new(),
            )));
        }
    }
    element("a:rPr", &attr_refs, children)
}

fn align_value(align: TextAlign) -> &'static str {
    match align {
        TextAlign::Left => "l",
        TextAlign::Center => "ctr",
        TextAlign::Right => "r",
    }
}

fn bool_value(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

fn max_cnvpr_id(element: &XmlElement) -> Option<i64> {
    let mut max_id = None;
    if element.name.local_name == "cNvPr"
        && let Some(id) = attr(element, "id").and_then(|value| value.parse::<i64>().ok())
    {
        max_id = Some(id);
    }
    for child in element.children.iter().filter_map(XmlNode::as_element) {
        max_id = max_id.max(max_cnvpr_id(child));
    }
    max_id
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

fn attr<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local_name == local_name)
        .map(|attribute| attribute.value.as_str())
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
use pptx_compose_core::opc::part_name::PartName;

#[cfg(test)]
#[test]
fn rejects_bounds_above_emu_max_without_writing() {
    let slide_part = PartName::from_zip_entry("ppt/slides/slide1.xml").expect("valid slide part");
    let slide_bytes =
        br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/></p:spTree></p:cSld></p:sld>"#.to_vec();
    let mut package = Package::new();
    package
        .insert_zip_entry("ppt/slides/slide1.xml", slide_bytes.clone())
        .expect("slide inserted");
    let target = ResolvedSlide {
        slide_id: "slide-1".to_owned(),
        part: slide_part.clone(),
    };
    let operation = AddTextBox {
        operation_id: "op-bounds".to_owned(),
        slide_id: "slide-1".to_owned(),
        text: "Hello".to_owned(),
        bounds: Bounds {
            x: 0,
            y: 0,
            cx: crate::operations::bounds::MAX_EMU_COORDINATE + 1,
            cy: 1,
        },
        name: None,
        alt_text: None,
        style: None,
        insert: None,
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
fn builds_template() {
    let slide_part = PartName::from_zip_entry("ppt/slides/slide1.xml").expect("valid slide part");
    let mut package = Package::new();
    package
            .insert_zip_entry(
                "ppt/slides/slide1.xml",
                br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="9" name="Existing"/></p:nvSpPr></p:sp></p:spTree></p:cSld></p:sld>"#.to_vec(),
            )
            .expect("slide inserted");
    let target = ResolvedSlide {
        slide_id: "slide-1".to_owned(),
        part: slide_part.clone(),
    };
    let operation = AddTextBox {
        operation_id: "op-1".to_owned(),
        slide_id: "slide-1".to_owned(),
        text: "Hello\nWorld".to_owned(),
        bounds: Bounds {
            x: 1,
            y: 2,
            cx: 3,
            cy: 4,
        },
        name: Some("Agent text".to_owned()),
        alt_text: Some("Readable description".to_owned()),
        style: Some(TextBoxStyle {
            font_size_pt: Some(18),
            bold: Some(true),
            italic: Some(false),
            font_family: Some("Aptos".to_owned()),
            color_hex: Some("112233".to_owned()),
            align: Some(TextAlign::Center),
            extra: Default::default(),
        }),
        insert: None,
    };

    let effects = operation
        .apply(&mut package, &target)
        .expect("text box is inserted");

    assert_eq!(effects.changed_parts, vec!["ppt/slides/slide1.xml"]);
    assert_eq!(effects.created_element_ids, vec!["slide-1:shape-10"]);
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
    assert!(
        slide_xml.contains(r#"<p:cNvPr id="10" name="Agent text" descr="Readable description"/>"#)
    );
    assert!(slide_xml.contains(r#"<p:cNvSpPr txBox="1"/><p:nvPr/>"#));
    assert!(slide_xml.contains(r#"<a:off x="1" y="2"/><a:ext cx="3" cy="4"/>"#));
    assert!(slide_xml.contains(r#"<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>"#));
    assert!(
        slide_xml.contains(
            r#"<a:bodyPr wrap="square" rtlCol="0"><a:spAutoFit/></a:bodyPr><a:lstStyle/>"#
        )
    );
    assert!(slide_xml.contains(r#"<a:p><a:pPr algn="ctr"/><a:r><a:rPr lang="en-US" dirty="0" sz="1800" b="1" i="0"><a:solidFill><a:srgbClr val="112233"/></a:solidFill><a:latin typeface="Aptos"/></a:rPr><a:t>Hello</a:t></a:r></a:p>"#));
    assert!(slide_xml.contains(r#"<a:t>World</a:t>"#));

    let unknown_style = serde_json::from_value::<AddTextBoxOperation>(serde_json::json!({
        "operation_id": "op-unknown",
        "slide_id": "slide-1",
        "text": "Bad style",
        "bounds": { "x": 0, "y": 0, "cx": 1, "cy": 1 },
        "style": { "shadow": true }
    }))
    .expect("unknown style keys are preserved for operation validation");
    let error = AddTextBox::from(&unknown_style)
        .validate()
        .expect_err("unknown style key is unsupported");
    assert_eq!(error.code(), ErrorCode::UnsupportedEdit);

    let invalid_bounds = AddTextBox {
        operation_id: "op-bounds".to_owned(),
        bounds: Bounds {
            x: 0,
            y: 0,
            cx: 0,
            cy: 1,
        },
        ..operation
    };
    let error = invalid_bounds
        .validate()
        .expect_err("zero width is invalid");
    assert_eq!(error.code(), ErrorCode::InvalidBounds);
}

#[cfg(test)]
#[test]
fn declares_missing_slide_namespaces_before_inserting_text_box() {
    let slide_part = PartName::from_zip_entry("ppt/slides/slide1.xml").expect("valid slide part");
    let mut package = Package::new();
    package
        .insert_zip_entry(
            "ppt/slides/slide1.xml",
            br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/></p:spTree></p:cSld></p:sld>"#.to_vec(),
        )
        .expect("slide inserted");
    let target = ResolvedSlide {
        slide_id: "slide-1".to_owned(),
        part: slide_part.clone(),
    };
    let operation = AddTextBox {
        operation_id: "op-namespaces".to_owned(),
        slide_id: "slide-1".to_owned(),
        text: "Hello".to_owned(),
        bounds: Bounds {
            x: 1,
            y: 2,
            cx: 3,
            cy: 4,
        },
        name: None,
        alt_text: None,
        style: None,
        insert: None,
    };

    operation
        .apply(&mut package, &target)
        .expect("text box is inserted");

    let slide_xml = String::from_utf8(
        package
            .parts()
            .get(&slide_part)
            .expect("slide still exists")
            .bytes()
            .to_vec(),
    )
    .expect("slide XML is UTF-8");
    parse_document(slide_xml.as_bytes()).expect("inserted slide XML remains well formed");
    assert_eq!(
        slide_xml,
        r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="1" name="TextBox 1"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="1" y="2"/><a:ext cx="3" cy="4"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr><p:txBody><a:bodyPr wrap="square" rtlCol="0"><a:spAutoFit/></a:bodyPr><a:lstStyle/><a:p><a:r><a:rPr lang="en-US" dirty="0"/><a:t>Hello</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
    );
}

#[cfg(test)]
#[test]
fn preserves_existing_slide_namespaces_without_duplicates() {
    let slide_part = PartName::from_zip_entry("ppt/slides/slide1.xml").expect("valid slide part");
    let mut package = Package::new();
    package
        .insert_zip_entry(
            "ppt/slides/slide1.xml",
            br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/></p:spTree></p:cSld></p:sld>"#.to_vec(),
        )
        .expect("slide inserted");
    let target = ResolvedSlide {
        slide_id: "slide-1".to_owned(),
        part: slide_part.clone(),
    };
    let operation = AddTextBox {
        operation_id: "op-existing-namespaces".to_owned(),
        slide_id: "slide-1".to_owned(),
        text: "Hello".to_owned(),
        bounds: Bounds {
            x: 1,
            y: 2,
            cx: 3,
            cy: 4,
        },
        name: None,
        alt_text: None,
        style: None,
        insert: None,
    };

    operation
        .apply(&mut package, &target)
        .expect("text box is inserted");

    let slide_xml = String::from_utf8(
        package
            .parts()
            .get(&slide_part)
            .expect("slide still exists")
            .bytes()
            .to_vec(),
    )
    .expect("slide XML is UTF-8");
    assert_eq!(slide_xml.matches("xmlns:p=").count(), 1);
    assert_eq!(slide_xml.matches("xmlns:a=").count(), 1);
    assert_eq!(slide_xml.matches("xmlns:r=").count(), 1);
    assert!(slide_xml.contains(r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#));
}
