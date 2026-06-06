use pptx_compose_core::{
    error::{Error, ErrorCode, ErrorLocation, Result},
    opc::{
        package::Package,
        part_name::PartName,
        relationships::{Relationship, RelationshipSet, RelationshipSource, TargetMode},
    },
    pptx::{
        ids::{ElementKind, SpTreePath, agent_element_id},
        media::{IMAGE_REL_TYPE, next_media_part_name},
    },
    xml::{
        document::{QualifiedName, XmlAttribute, XmlDocument, XmlElement, XmlNode},
        namespaces::NamespaceBinding,
        parser::parse_document,
        writer::{WriteMode, WriteOptions, write_document},
    },
};
use pptx_compose_json::schemas::OperationTarget;

use crate::{
    media_inputs::MediaInputs,
    operations::ResolvedSlide,
    patch::{AddImageOperation, Bounds, ImageDedupe, ImageFit, PatchEffects},
};

const P_NS: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const RELS_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddImage {
    pub operation_id: String,
    pub slide_id: String,
    pub media_ref: String,
    pub content_type: String,
    pub bounds: Bounds,
    pub name: Option<String>,
    pub alt_text: Option<String>,
    pub fit: ImageFit,
    pub dedupe: ImageDedupe,
}

impl From<&AddImageOperation> for AddImage {
    fn from(operation: &AddImageOperation) -> Self {
        Self {
            operation_id: operation.operation_id.clone(),
            slide_id: operation.slide_id.clone(),
            media_ref: operation.media_ref.clone(),
            content_type: operation.content_type.clone(),
            bounds: operation.bounds.clone(),
            name: operation.name.clone(),
            alt_text: operation.alt_text.clone(),
            fit: operation.fit.unwrap_or(ImageFit::Stretch),
            dedupe: operation.dedupe.unwrap_or(ImageDedupe::Never),
        }
    }
}

impl AddImage {
    pub fn validate(&self, media_inputs: &MediaInputs) -> Result<()> {
        validate_bounds(&self.bounds).map_err(|error| error.with_location(self.location(None)))?;
        if self.fit != ImageFit::Stretch {
            return Err(Error::new(
                ErrorCode::UnsupportedEdit,
                "add_image supports only fit: stretch in V1.",
            )
            .with_location(self.location(None)));
        }
        if self.dedupe != ImageDedupe::Never {
            return Err(Error::new(
                ErrorCode::UnsupportedEdit,
                "add_image supports only dedupe: never in V1.",
            )
            .with_location(self.location(None)));
        }
        let media = media_inputs
            .resolve(&self.media_ref)
            .map_err(|error| error.with_location(self.location(None)))?;
        if media.content_type != self.content_type {
            return Err(Error::new(
                ErrorCode::UnsupportedMediaType,
                format!(
                    "add_image content_type `{}` does not match bound media_ref `{}` content type `{}`.",
                    self.content_type, self.media_ref, media.content_type
                ),
            )
            .with_location(self.location(None)));
        }
        extension_for_content_type(&self.content_type)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::UnsupportedMediaType,
                    format!(
                        "add_image content_type `{}` is not supported in V1.",
                        self.content_type
                    ),
                )
            })
            .map(|_| ())
            .map_err(|error| error.with_location(self.location(None)))
    }

    pub fn apply(
        &self,
        package: &mut Package,
        target: &ResolvedSlide,
        media_inputs: &MediaInputs,
    ) -> Result<PatchEffects> {
        self.validate(media_inputs)?;
        if target.slide_id != self.slide_id {
            return Err(Error::new(
                ErrorCode::SelectorGuardFailed,
                "Resolved slide does not match add_image slide_id.",
            )
            .with_location(self.location(None)));
        }

        let media = media_inputs
            .resolve(&self.media_ref)
            .map_err(|error| error.with_location(self.location(None)))?;
        let extension = extension_for_content_type(&self.content_type).ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedMediaType,
                format!(
                    "add_image content_type `{}` is not supported in V1.",
                    self.content_type
                ),
            )
            .with_location(self.location(None))
        })?;
        let media_part = next_media_part_name(package.parts(), extension);
        package
            .insert_zip_entry(media_part.zip_entry_name(), media.bytes)
            .map_err(|error| error.with_location(self.location(None)))?;
        ensure_content_type(package, &media_part, extension, &self.content_type);

        let slide_part = target.part.clone();
        let target_value = relative_target(&slide_part, &media_part);
        let rel_id = add_slide_relationship(package, &slide_part, &target_value, &media_part)
            .map_err(|error| error.with_location(self.location(None)))?;

        let created_element_id = insert_picture(package, target, self, &rel_id)?;
        let rels_part = rels_part_name_for(&slide_part)?;
        let content_types_part = content_types_part()?;
        package.mark_dirty(slide_part.clone());
        package.mark_dirty(rels_part.clone());
        package.mark_dirty(content_types_part);
        package.mark_dirty(media_part.clone());

        Ok(PatchEffects {
            changed_parts: vec![
                slide_part.zip_entry_name().to_owned(),
                rels_part.zip_entry_name().to_owned(),
                media_part.zip_entry_name().to_owned(),
                "[Content_Types].xml".to_owned(),
            ],
            target: Some(OperationTarget {
                slide_id: target.slide_id.clone(),
                element_id: created_element_id.clone(),
                part: slide_part.zip_entry_name().to_owned(),
            }),
            created_element_ids: vec![created_element_id],
            warnings: Vec::new(),
        })
    }

    fn location(&self, element_id: Option<String>) -> ErrorLocation {
        ErrorLocation {
            slide_id: Some(self.slide_id.clone()),
            element_id,
            operation_id: Some(self.operation_id.clone()),
            operation: Some("add_image".to_owned()),
            ..ErrorLocation::default()
        }
    }
}

fn validate_bounds(bounds: &Bounds) -> Result<()> {
    if bounds.x < 0 || bounds.y < 0 || bounds.cx <= 0 || bounds.cy <= 0 {
        return Err(Error::new(
            ErrorCode::InvalidBounds,
            "Bounds require x/y >= 0 and cx/cy > 0.",
        ));
    }
    Ok(())
}

fn ensure_content_type(
    package: &mut Package,
    media_part: &PartName,
    extension: &str,
    content_type: &str,
) {
    if package
        .content_types()
        .default_for_ext(extension)
        .is_some_and(|existing| existing == content_type)
    {
        return;
    }

    if package.content_types().default_for_ext(extension).is_none() {
        package
            .content_types_mut()
            .insert_default(extension, content_type);
    } else {
        package
            .content_types_mut()
            .insert_override(media_part.clone(), content_type);
    }
}

fn add_slide_relationship(
    package: &mut Package,
    slide_part: &PartName,
    target_value: &str,
    media_part: &PartName,
) -> Result<String> {
    let rel_id = package
        .relationships()
        .set_for(slide_part)
        .map_or_else(|| "rId1".to_owned(), RelationshipSet::allocate_id);
    let relationship = Relationship {
        source: RelationshipSource::Part(slide_part.clone()),
        id: rel_id.clone(),
        rel_type: IMAGE_REL_TYPE.to_owned(),
        target: target_value.to_owned(),
        mode: TargetMode::Internal,
        target_mode: TargetMode::Internal,
        resolved_target: Some(media_part.clone()),
    };
    package.push_relationship(relationship);

    let rels_part = rels_part_name_for(slide_part)?;
    if let Some(part) = package.parts_mut().get_mut(&rels_part) {
        let mut document = parse_document(part.bytes())?;
        append_relationship(&mut document, &rel_id, target_value)?;
        *part.bytes_mut() = write_document(
            &document,
            &WriteOptions {
                mode: WriteMode::Preserve,
            },
        )?;
    } else {
        let document = new_relationships_document(&rel_id, target_value);
        package.insert_zip_entry(
            rels_part.zip_entry_name(),
            write_document(
                &document,
                &WriteOptions {
                    mode: WriteMode::Preserve,
                },
            )?,
        )?;
    }

    Ok(rel_id)
}

fn append_relationship(document: &mut XmlDocument, rel_id: &str, target_value: &str) -> Result<()> {
    let root = root_element_mut(document).ok_or_else(|| {
        Error::malformed_xml("Relationship part does not contain a root element.")
    })?;
    if root.name.local_name != "Relationships" {
        return Err(Error::unsupported_package(
            "Relationship part root element is not Relationships.",
        ));
    }
    root.children
        .push(XmlNode::Element(relationship_element(rel_id, target_value)));
    Ok(())
}

fn new_relationships_document(rel_id: &str, target_value: &str) -> XmlDocument {
    let mut root = element(
        "Relationships",
        &[],
        vec![XmlNode::Element(relationship_element(rel_id, target_value))],
    );
    root.namespaces.push(NamespaceBinding::default(RELS_NS));
    XmlDocument {
        declaration: None,
        nodes: vec![XmlNode::Element(root)],
    }
}

fn relationship_element(rel_id: &str, target_value: &str) -> XmlElement {
    element(
        "Relationship",
        &[
            ("Id", rel_id),
            ("Type", IMAGE_REL_TYPE),
            ("Target", target_value),
        ],
        Vec::new(),
    )
}

fn insert_picture(
    package: &mut Package,
    target: &ResolvedSlide,
    operation: &AddImage,
    rel_id: &str,
) -> Result<String> {
    let part_name = target.part.clone();
    let part = package.parts_mut().get_mut(&part_name).ok_or_else(|| {
        Error::new(
            ErrorCode::SelectorNotFound,
            format!("Target slide part {part_name} was not found."),
        )
        .with_location(operation.location(None))
    })?;

    let mut document = parse_document(part.bytes()).map_err(|source| {
        Error::with_source(
            source.code(),
            format!("Could not parse target slide part {part_name}."),
            source,
        )
        .with_location(operation.location(None))
    })?;
    let element_id = insert_picture_xml(&mut document, operation, rel_id)?;
    *part.bytes_mut() = write_document(
        &document,
        &WriteOptions {
            mode: WriteMode::Preserve,
        },
    )?;
    Ok(element_id)
}

fn insert_picture_xml(
    document: &mut XmlDocument,
    operation: &AddImage,
    rel_id: &str,
) -> Result<String> {
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
        .unwrap_or_else(|| format!("Picture {id}"));
    let picture = picture_shape(
        id,
        &name,
        operation.alt_text.as_deref(),
        &operation.bounds,
        rel_id,
    );
    sp_tree.children.push(XmlNode::Element(picture));

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
        ElementKind::Picture,
        &path,
    ))
}

fn ensure_slide_namespaces(root: &mut XmlElement) {
    ensure_namespace(root, Some("p"), P_NS);
    ensure_namespace(root, Some("a"), A_NS);
    ensure_namespace(root, Some("r"), R_NS);
}

fn ensure_namespace(root: &mut XmlElement, prefix: Option<&str>, uri: &str) {
    if root.namespaces.resolve_prefix(prefix) == Some(uri) {
        return;
    }
    if let Some(prefix) = prefix {
        root.namespaces
            .push(NamespaceBinding::prefixed(prefix, uri));
    } else {
        root.namespaces.push(NamespaceBinding::default(uri));
    }
}

fn picture_shape(
    id: i64,
    name: &str,
    alt_text: Option<&str>,
    bounds: &Bounds,
    rel_id: &str,
) -> XmlElement {
    element(
        "p:pic",
        &[],
        vec![
            node(element(
                "p:nvPicPr",
                &[],
                vec![
                    node(cnv_pr(id, name, alt_text)),
                    node(element(
                        "p:cNvPicPr",
                        &[],
                        vec![node(element(
                            "a:picLocks",
                            &[("noChangeAspect", "1")],
                            Vec::new(),
                        ))],
                    )),
                    node(element("p:nvPr", &[], Vec::new())),
                ],
            )),
            node(element(
                "p:blipFill",
                &[],
                vec![
                    node(element("a:blip", &[("r:embed", rel_id)], Vec::new())),
                    node(element(
                        "a:stretch",
                        &[],
                        vec![node(element("a:fillRect", &[], Vec::new()))],
                    )),
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
                                &[("x", &bounds.x.to_string()), ("y", &bounds.y.to_string())],
                                Vec::new(),
                            )),
                            node(element(
                                "a:ext",
                                &[
                                    ("cx", &bounds.cx.to_string()),
                                    ("cy", &bounds.cy.to_string()),
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

fn root_element_mut(document: &mut XmlDocument) -> Option<&mut XmlElement> {
    document.nodes.iter_mut().find_map(node_element_mut)
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

fn extension_for_content_type(content_type: &str) -> Option<&'static str> {
    match content_type {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        _ => None,
    }
}

fn relative_target(source_part: &PartName, target_part: &PartName) -> String {
    let source_segments = source_part
        .as_str()
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();
    let target_segments = target_part
        .as_str()
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();
    let source_dir = &source_segments[..source_segments.len().saturating_sub(1)];
    let mut common = 0_usize;
    while common < source_dir.len()
        && common < target_segments.len()
        && source_dir[common] == target_segments[common]
    {
        common += 1;
    }

    let mut relative = Vec::new();
    relative.extend(std::iter::repeat_n(
        "..",
        source_dir.len().saturating_sub(common),
    ));
    relative.extend(target_segments[common..].iter().copied());
    relative.join("/")
}

fn rels_part_name_for(part_name: &PartName) -> Result<PartName> {
    let path = part_name.as_str();
    let Some((directory, file_name)) = path.rsplit_once('/') else {
        return PartName::from_zip_entry(format!("/_rels/{path}.rels").as_str());
    };
    let rels_path = if directory.is_empty() {
        format!("/_rels/{file_name}.rels")
    } else {
        format!("{directory}/_rels/{file_name}.rels")
    };
    PartName::from_zip_entry(rels_path.as_str())
}

fn content_types_part() -> Result<PartName> {
    PartName::from_zip_entry("[Content_Types].xml")
}

#[cfg(test)]
use std::collections::HashMap;

#[cfg(test)]
use pptx_compose_core::opc::relationships::resolve_internal_target;

#[cfg(test)]
use crate::media_inputs::{MediaBinding, MediaSource};

#[cfg(test)]
#[test]
fn wires_part_rel_ctype() {
    let slide_part = test_part("ppt/slides/slide1.xml");
    let mut package = Package::new();
    package
        .insert_zip_entry("ppt/slides/slide1.xml", slide_xml().as_bytes().to_vec())
        .expect("slide inserted");
    package
        .insert_zip_entry(
            "ppt/slides/_rels/slide1.xml.rels",
            rels_xml().as_bytes().to_vec(),
        )
        .expect("rels inserted");
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(slide_part.clone()),
        "rId2",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout",
        "../slideLayouts/slideLayout1.xml",
    ));

    let operation = AddImage {
        operation_id: "op-1".to_owned(),
        slide_id: "slide-1".to_owned(),
        media_ref: "hero".to_owned(),
        content_type: "image/png".to_owned(),
        bounds: Bounds {
            x: 10,
            y: 20,
            cx: 300,
            cy: 400,
        },
        name: Some("Hero".to_owned()),
        alt_text: Some("Hero image".to_owned()),
        fit: ImageFit::Stretch,
        dedupe: ImageDedupe::Never,
    };
    let target = ResolvedSlide {
        slide_id: "slide-1".to_owned(),
        part: slide_part.clone(),
    };

    let effects = operation
        .apply(&mut package, &target, &media_inputs())
        .expect("image is added");

    let media_part = test_part("ppt/media/image1.png");
    assert!(package.parts().get(&media_part).is_some());
    assert_eq!(
        package.content_types().resolve(&media_part),
        Some("image/png")
    );

    let relationship = package
        .relationships()
        .set_for(&slide_part)
        .and_then(|set| set.get("rId3"))
        .expect("new relationship exists");
    assert_eq!(relationship.rel_type, IMAGE_REL_TYPE);
    assert_eq!(relationship.target, "../media/image1.png");
    assert_eq!(
        relationship
            .resolved_target
            .as_ref()
            .expect("resolved target"),
        &media_part
    );
    assert_eq!(
        resolve_internal_target(&relationship.source, &relationship.target)
            .expect("relationship target resolves"),
        media_part
    );

    let slide_xml = String::from_utf8(
        package
            .parts()
            .get(&slide_part)
            .expect("slide exists")
            .bytes()
            .to_vec(),
    )
    .expect("slide XML UTF-8");
    assert!(slide_xml.contains(r#"<p:pic>"#));
    assert!(slide_xml.contains(r#"<p:cNvPr id="6" name="Hero" descr="Hero image"/>"#));
    assert!(slide_xml.contains(r#"<a:blip r:embed="rId3"/>"#));
    assert!(slide_xml.contains(r#"<a:off x="10" y="20"/><a:ext cx="300" cy="400"/>"#));

    let rels_xml = String::from_utf8(
        package
            .parts()
            .get(&test_part("ppt/slides/_rels/slide1.xml.rels"))
            .expect("rels exists")
            .bytes()
            .to_vec(),
    )
    .expect("rels XML UTF-8");
    assert!(rels_xml.contains(r#"Id="rId3""#));
    assert!(rels_xml.contains(r#"Target="../media/image1.png""#));

    assert_eq!(effects.created_element_ids, vec!["slide-1:pic-4"]);
    assert_eq!(
        effects.changed_parts,
        vec![
            "ppt/slides/slide1.xml",
            "ppt/slides/_rels/slide1.xml.rels",
            "ppt/media/image1.png",
            "[Content_Types].xml"
        ]
    );
    assert!(package.dirty_parts().contains(&slide_part));
    assert!(package.dirty_parts().contains(&media_part));

    let missing = AddImage {
        operation_id: "op-missing".to_owned(),
        media_ref: "missing".to_owned(),
        ..operation
    };
    let error = missing
        .validate(&media_inputs())
        .expect_err("missing media_ref is rejected");
    assert_eq!(error.code(), ErrorCode::MissingMediaRef);
}

#[cfg(test)]
fn media_inputs() -> MediaInputs {
    let mut bindings = HashMap::new();
    bindings.insert(
        "hero".to_owned(),
        MediaBinding {
            content_type: "image/png".to_owned(),
            declared_sha256: None,
            declared_byte_length: None,
            source: MediaSource::Bytes(one_by_one_png()),
        },
    );
    MediaInputs::new(bindings)
}

#[cfg(test)]
fn test_part(name: &str) -> PartName {
    PartName::from_zip_entry(name).expect("valid fixture part")
}

#[cfg(test)]
fn slide_xml() -> &'static str {
    r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="5" name="Existing"/></p:nvSpPr></p:sp></p:spTree></p:cSld></p:sld>"#
}

#[cfg(test)]
fn rels_xml() -> &'static str {
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/></Relationships>"#
}

#[cfg(test)]
fn one_by_one_png() -> Vec<u8> {
    vec![
        0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0x00, 0x00, 0x00, 0x0d, b'I', b'H',
        b'D', b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00,
        0x1f, 0x15, 0xc4, 0x89,
    ]
}
