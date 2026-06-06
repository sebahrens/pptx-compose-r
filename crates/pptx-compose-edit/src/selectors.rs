use pptx_compose_core::{
    error::{Error, ErrorCode, ErrorLocation, Result},
    opc::part_name::PartName,
    pptx::{
        ids::{ElementKind, agent_element_id, index_sp_tree},
        presentation::PresentationDocument,
        shape::read_shape,
        text::read_text_body,
    },
    provenance::{
        fingerprint::{FingerprintInput, fingerprint},
        text_hash,
    },
    xml::{
        document::{XmlDocument, XmlElement, XmlNode},
        parser::parse_document,
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::operations::{ResolvedElement, ResolvedMediaPart, ResolvedSlide, ResolvedTarget};

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Selector {
    ElementId {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        guards: Option<Guards>,
    },
    SlideId {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        guards: Option<Guards>,
    },
    MediaPart {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        part: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        guards: Option<Guards>,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Guards {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slide_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

pub fn resolve(model: &PresentationDocument, selector: &Selector) -> Result<ResolvedTarget> {
    match selector {
        Selector::ElementId { id, guards } => resolve_element(model, id, guards.as_ref()),
        Selector::SlideId { id, guards } => resolve_slide(model, id, guards.as_ref()),
        Selector::MediaPart { part, guards } => {
            resolve_media_part(model, part.as_deref(), guards.as_ref())
        }
    }
}

fn resolve_slide(
    model: &PresentationDocument,
    id: &str,
    guards: Option<&Guards>,
) -> Result<ResolvedTarget> {
    let matches = model
        .slides()
        .iter()
        .filter(|slide| slide.agent_id() == id)
        .collect::<Vec<_>>();
    let slide = exactly_one(matches, "slide", id, None)?;
    let target = ResolvedSlide {
        slide_id: slide.agent_id(),
        part: slide.part_name.clone(),
    };
    if let Some(guards) = guards {
        guard_eq(
            "slide_id",
            guards.slide_id.as_deref(),
            &target.slide_id,
            None,
        )?;
        guard_eq(
            "part",
            guards.part.as_deref(),
            target.part.zip_entry_name(),
            None,
        )?;
        reject_inapplicable_guard("kind", guards.kind.as_deref(), None)?;
        reject_inapplicable_guard("text_hash", guards.text_hash.as_deref(), None)?;
        reject_inapplicable_guard("fingerprint", guards.fingerprint.as_deref(), None)?;
    }
    Ok(ResolvedTarget::Slide(target))
}

fn resolve_element(
    model: &PresentationDocument,
    id: &str,
    guards: Option<&Guards>,
) -> Result<ResolvedTarget> {
    let mut matches = Vec::new();
    for (slide_index, slide) in model.slides().iter().enumerate() {
        let slide_id = slide.agent_id();
        let Some(document) = slide_document(model, &slide.part_name)? else {
            continue;
        };
        let Some(sp_tree) = document
            .root_element()
            .and_then(|root| first_descendant(root, "spTree"))
        else {
            continue;
        };
        for (path, kind) in index_sp_tree(sp_tree) {
            let Some(element) = element_at_path(sp_tree, &path.sp_tree_path) else {
                continue;
            };
            let shape = read_shape(element, path.clone());
            let element_id = agent_element_id(&slide_id, kind, shape.cnvpr_id, &path);
            if element_id != id {
                continue;
            }
            let text_hash = element_text_hash(element);
            let fingerprint = fingerprint(&FingerprintInput {
                kind,
                part: slide.part_name.clone(),
                sp_tree_path: path.sp_tree_path.clone(),
                group_path: path.group_path.clone(),
                cnvpr_id: shape.cnvpr_id,
                text_hash: text_hash.clone(),
            });
            matches.push(ResolvedElement {
                slide_id: pptx_compose_core::pptx::ids::slide_agent_id(slide_index),
                element_id,
                kind,
                part: slide.part_name.clone(),
                sp_tree_path: path.sp_tree_path,
                group_path: path.group_path,
                cnvpr_id: shape.cnvpr_id,
                text_hash,
                fingerprint,
            });
        }
    }

    let target = exactly_one(matches, "element", id, Some(id))?;
    if let Some(guards) = guards {
        guard_eq(
            "slide_id",
            guards.slide_id.as_deref(),
            &target.slide_id,
            Some(&target.element_id),
        )?;
        guard_eq(
            "kind",
            guards.kind.as_deref(),
            kind_name(target.kind),
            Some(&target.element_id),
        )?;
        guard_eq(
            "part",
            guards.part.as_deref(),
            target.part.zip_entry_name(),
            Some(&target.element_id),
        )?;
        guard_optional_eq(
            "text_hash",
            guards.text_hash.as_deref(),
            target.text_hash.as_deref(),
            Some(&target.element_id),
        )?;
        guard_eq(
            "fingerprint",
            guards.fingerprint.as_deref(),
            &target.fingerprint,
            Some(&target.element_id),
        )?;
    }
    Ok(ResolvedTarget::Element(target))
}

fn resolve_media_part(
    model: &PresentationDocument,
    part: Option<&str>,
    guards: Option<&Guards>,
) -> Result<ResolvedTarget> {
    let matches = model
        .package()
        .parts()
        .iter()
        .filter(|candidate| {
            let name = candidate.name().zip_entry_name();
            name.starts_with("ppt/media/")
                && part.is_none_or(|part| part == name || part == candidate.name().as_str())
        })
        .map(|candidate| ResolvedMediaPart {
            part: candidate.name().clone(),
        })
        .collect::<Vec<_>>();

    let target = exactly_one(matches, "media part", part.unwrap_or("<any>"), None)?;
    if let Some(guards) = guards {
        guard_eq(
            "part",
            guards.part.as_deref(),
            target.part.zip_entry_name(),
            None,
        )?;
        reject_inapplicable_guard("slide_id", guards.slide_id.as_deref(), None)?;
        reject_inapplicable_guard("kind", guards.kind.as_deref(), None)?;
        reject_inapplicable_guard("text_hash", guards.text_hash.as_deref(), None)?;
        reject_inapplicable_guard("fingerprint", guards.fingerprint.as_deref(), None)?;
    }
    Ok(ResolvedTarget::MediaPart(target))
}

fn slide_document(
    model: &PresentationDocument,
    part_name: &PartName,
) -> Result<Option<XmlDocument>> {
    let Some(part) = model.package().parts().get(part_name) else {
        return Ok(None);
    };
    let document = parse_document(part.bytes()).map_err(|source| {
        Error::with_source(
            source.code(),
            format!("Could not parse slide part {part_name}."),
            source,
        )
    })?;
    Ok(Some(document))
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

fn element_text_hash(element: &XmlElement) -> Option<String> {
    let tx_body = first_descendant(element, "txBody")?;
    let text_body = read_text_body(tx_body);
    Some(text_hash::text_hash(&text_body.normalized))
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

fn exactly_one<T>(
    matches: Vec<T>,
    target_type: &str,
    selector: &str,
    element_id: Option<&str>,
) -> Result<T> {
    let count = matches.len();
    if count == 1 {
        return matches.into_iter().next().ok_or_else(|| {
            selector_not_found(
                format!("Selector for {target_type} {selector} resolved unexpectedly empty."),
                element_id,
            )
        });
    }

    if count == 0 {
        Err(selector_not_found(
            format!("Selector for {target_type} {selector} resolved to 0 targets."),
            element_id,
        ))
    } else {
        Err(selector_ambiguous(
            format!(
                "Selector for {target_type} {selector} resolved to {count} targets; expected exactly one."
            ),
            element_id,
        ))
    }
}

fn guard_eq(
    guard_name: &str,
    expected: Option<&str>,
    actual: &str,
    element_id: Option<&str>,
) -> Result<()> {
    if expected.is_some_and(|expected| expected != actual) {
        return Err(selector_guard_failed(
            format!("Selector guard {guard_name} did not match the current target."),
            element_id,
        ));
    }
    Ok(())
}

fn guard_optional_eq(
    guard_name: &str,
    expected: Option<&str>,
    actual: Option<&str>,
    element_id: Option<&str>,
) -> Result<()> {
    if let Some(expected) = expected
        && actual != Some(expected)
    {
        return Err(selector_guard_failed(
            format!("Selector guard {guard_name} did not match the current target."),
            element_id,
        ));
    }
    Ok(())
}

fn reject_inapplicable_guard(
    guard_name: &str,
    value: Option<&str>,
    element_id: Option<&str>,
) -> Result<()> {
    if value.is_some() {
        return Err(selector_guard_failed(
            format!("Selector guard {guard_name} is not applicable to this target type."),
            element_id,
        ));
    }
    Ok(())
}

fn selector_guard_failed(message: String, element_id: Option<&str>) -> Error {
    Error::new(ErrorCode::SelectorGuardFailed, message).with_location(ErrorLocation {
        element_id: element_id.map(str::to_owned),
        ..ErrorLocation::default()
    })
}

fn selector_not_found(message: String, element_id: Option<&str>) -> Error {
    Error::new(ErrorCode::SelectorNotFound, message).with_location(ErrorLocation {
        element_id: element_id.map(str::to_owned),
        ..ErrorLocation::default()
    })
}

fn selector_ambiguous(message: String, element_id: Option<&str>) -> Error {
    Error::new(ErrorCode::SelectorAmbiguous, message).with_location(ErrorLocation {
        element_id: element_id.map(str::to_owned),
        ..ErrorLocation::default()
    })
}

const fn kind_name(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::TextBox => "text_box",
        ElementKind::Shape => "shape",
        ElementKind::Picture => "image",
        ElementKind::Group => "group",
        ElementKind::GraphicFrame => "graphic_frame",
        ElementKind::Connector => "connector",
        ElementKind::Other => "other",
    }
}

#[cfg(test)]
use pptx_compose_core::{
    opc::{
        package::Package,
        relationships::{Relationship, RelationshipSource},
    },
    pptx::presentation::PresentationDocument as TestPresentationDocument,
};

#[cfg(test)]
#[test]
fn resolve_and_guard() {
    let document = fixture_document();
    let initial = resolve(
        &document,
        &Selector::ElementId {
            id: "slide-1:shape-4".to_owned(),
            guards: None,
        },
    )
    .expect("element resolves");
    let ResolvedTarget::Element(element) = initial else {
        panic!("expected element target");
    };

    let matched = resolve(
        &document,
        &Selector::ElementId {
            id: element.element_id.clone(),
            guards: Some(Guards {
                slide_id: Some(element.slide_id.clone()),
                kind: Some("text_box".to_owned()),
                part: Some("ppt/slides/slide1.xml".to_owned()),
                text_hash: element.text_hash.clone(),
                fingerprint: Some(element.fingerprint.clone()),
            }),
        },
    )
    .expect("guarded element resolves");
    assert_eq!(matched, ResolvedTarget::Element(element.clone()));

    let stale = resolve(
        &document,
        &Selector::ElementId {
            id: element.element_id,
            guards: Some(Guards {
                fingerprint: Some(
                    "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                        .to_owned(),
                ),
                ..Guards::default()
            }),
        },
    )
    .expect_err("stale fingerprint fails");
    assert_eq!(stale.code(), ErrorCode::SelectorGuardFailed);

    let ambiguous = resolve(
        &document,
        &Selector::MediaPart {
            part: None,
            guards: None,
        },
    )
    .expect_err("ambiguous media selector fails");
    assert_eq!(ambiguous.code(), ErrorCode::SelectorAmbiguous);
}

#[cfg(test)]
fn fixture_document() -> TestPresentationDocument {
    let mut package = Package::new();
    insert(&mut package, "ppt/presentation.xml", presentation_xml());
    insert(&mut package, "ppt/slides/slide1.xml", slide_xml());
    insert(&mut package, "ppt/media/image1.png", b"one");
    insert(&mut package, "ppt/media/image2.png", b"two");

    let presentation_part = part("ppt/presentation.xml");
    package.push_relationship(Relationship::internal(
        RelationshipSource::Package,
        "rOffice",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
        "ppt/presentation.xml",
    ));
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(presentation_part),
        "rSlide",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide",
        "slides/slide1.xml",
    ));

    TestPresentationDocument::open(package).expect("fixture presentation opens")
}

#[cfg(test)]
fn insert(package: &mut Package, name: &str, bytes: &[u8]) {
    package
        .insert_zip_entry(name, bytes.to_vec())
        .expect("fixture part inserts");
}

#[cfg(test)]
fn part(name: &str) -> pptx_compose_core::opc::part_name::PartName {
    pptx_compose_core::opc::part_name::PartName::from_zip_entry(name).expect("valid fixture part")
}

#[cfg(test)]
fn presentation_xml() -> &'static [u8] {
    br#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="256" r:id="rSlide"/></p:sldIdLst></p:presentation>"#
}

#[cfg(test)]
fn slide_xml() -> &'static [u8] {
    br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="4" name="Title 1"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:p><a:r><a:t>Quarterly Results</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
}
