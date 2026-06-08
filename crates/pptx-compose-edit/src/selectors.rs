use pptx_compose_core::{
    error::{Error, ErrorCode, ErrorLocation, Result},
    opc::part_name::PartName,
    opc::relationships::{RelationshipSource, TargetMode, resolve_internal_target},
    pptx::{
        ids::{ElementKind, agent_element_id, index_sp_tree},
        presentation::PresentationDocument,
        shape::read_shape,
        text::read_text_body,
    },
    provenance::{
        checksum::part_checksum,
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

use crate::operations::{
    ResolvedCoreProperties, ResolvedElement, ResolvedMediaPart, ResolvedNotesSlide, ResolvedSlide,
    ResolvedTarget, is_real_shape_tree_child,
};

const CORE_PROPERTIES_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";
const NOTES_SLIDE_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide";

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Selector {
    ElementId {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        guards: Option<Guards>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run: Option<RunSelector>,
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
    CoreProperties {
        part: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        guards: Option<Guards>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunSelector {
    pub paragraph_index: u32,
    pub run_index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_end_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_hash: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_checksum: Option<String>,
}

pub fn resolve(model: &PresentationDocument, selector: &Selector) -> Result<ResolvedTarget> {
    match selector {
        Selector::ElementId { id, guards, .. } => resolve_element(model, id, guards.as_ref()),
        Selector::SlideId { id, guards } => resolve_slide(model, id, guards.as_ref()),
        Selector::MediaPart { part, guards } => {
            resolve_media_part(model, part.as_deref(), guards.as_ref())
        }
        Selector::CoreProperties { part, guards } => {
            resolve_core_properties(model, part, guards.as_ref())
        }
    }
}

pub fn resolve_notes_slide(
    model: &PresentationDocument,
    selector: &Selector,
) -> Result<ResolvedTarget> {
    let ResolvedTarget::Slide(slide) = resolve(model, selector)? else {
        return Err(selector_guard_failed(
            "Selector did not resolve to a slide.".to_owned(),
            None,
            Some("slide_id"),
            Some("<different target type>"),
        ));
    };

    let relationship = model
        .package()
        .relationships()
        .iter()
        .find(|relationship| {
            relationship.source == RelationshipSource::Part(slide.part.clone())
                && relationship.rel_type == NOTES_SLIDE_REL_TYPE
        })
        .ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                "Target slide does not have a speaker-notes part; V1 does not create notes slides.",
            )
            .with_location(ErrorLocation {
                slide_id: Some(slide.slide_id.clone()),
                part: Some(slide.part.zip_entry_name().to_owned()),
                ..ErrorLocation::default()
            })
        })?;

    if relationship.target_mode != TargetMode::Internal {
        return Err(Error::new(
            ErrorCode::UnsupportedEdit,
            "Speaker-notes relationship must target an internal notes slide part.",
        )
        .with_location(ErrorLocation {
            slide_id: Some(slide.slide_id.clone()),
            part: Some(slide.part.zip_entry_name().to_owned()),
            ..ErrorLocation::default()
        }));
    }

    let notes_part = relationship.resolved_target.clone().map_or_else(
        || resolve_internal_target(&relationship.source, &relationship.target),
        Ok,
    )?;
    if model.package().parts().get(&notes_part).is_none() {
        return Err(Error::new(
            ErrorCode::UnsupportedEdit,
            format!("Speaker-notes part {notes_part} was not found."),
        )
        .with_location(ErrorLocation {
            slide_id: Some(slide.slide_id.clone()),
            part: Some(notes_part.zip_entry_name().to_owned()),
            ..ErrorLocation::default()
        }));
    }

    Ok(ResolvedTarget::NotesSlide(ResolvedNotesSlide {
        slide_id: slide.slide_id.clone(),
        slide_part: slide.part,
        notes_part,
        element_id: format!("{}:notes", slide.slide_id),
    }))
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
    let slide = exactly_one(matches, "slide", id, None, |slide| slide.agent_id())?;
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
        reject_inapplicable_guard("part_checksum", guards.part_checksum.as_deref(), None)?;
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
            let text_hash = element_text_hash(model, &slide.part_name, element, kind)?;
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

    let target = exactly_one(matches, "element", id, Some(id), |target| {
        target.element_id.clone()
    })?;
    if let Some(guards) = guards {
        guard_eq(
            "slide_id",
            guards.slide_id.as_deref(),
            &target.slide_id,
            Some(&target.element_id),
        )?;
        guard_kind(
            "kind",
            guards.kind.as_deref(),
            target.kind,
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
        reject_inapplicable_guard(
            "part_checksum",
            guards.part_checksum.as_deref(),
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

    let target = exactly_one(
        matches,
        "media part",
        part.unwrap_or("<any>"),
        None,
        |target| target.part.zip_entry_name().to_owned(),
    )?;
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
        reject_inapplicable_guard("part_checksum", guards.part_checksum.as_deref(), None)?;
    }
    Ok(ResolvedTarget::MediaPart(target))
}

fn resolve_core_properties(
    model: &PresentationDocument,
    part: &str,
    guards: Option<&Guards>,
) -> Result<ResolvedTarget> {
    let relationship = model
        .package()
        .relationships()
        .iter()
        .find(|relationship| {
            relationship.source
                == pptx_compose_core::opc::relationships::RelationshipSource::Package
                && relationship.rel_type == CORE_PROPERTIES_REL_TYPE
        })
        .ok_or_else(|| {
            selector_not_found(
                "Package root relationships do not contain a core-properties relationship."
                    .to_owned(),
                None,
            )
        })?;

    if relationship.target_mode != pptx_compose_core::opc::relationships::TargetMode::Internal {
        return Err(selector_not_found(
            "Core-properties relationship must be internal.".to_owned(),
            None,
        ));
    }

    let resolved = match &relationship.resolved_target {
        Some(resolved) => resolved.clone(),
        None => pptx_compose_core::opc::relationships::resolve_internal_target(
            &relationship.source,
            &relationship.target,
        )?,
    };
    let Some(core_part) = model.package().parts().get(&resolved) else {
        return Err(selector_not_found(
            format!("Core-properties part {resolved} was not found."),
            None,
        ));
    };

    guard_eq("part", Some(part), resolved.zip_entry_name(), None)?;
    if let Some(guards) = guards {
        guard_eq(
            "part",
            guards.part.as_deref(),
            resolved.zip_entry_name(),
            None,
        )?;
        guard_eq(
            "part_checksum",
            guards.part_checksum.as_deref(),
            &part_checksum(core_part.bytes()),
            None,
        )?;
        reject_inapplicable_guard("slide_id", guards.slide_id.as_deref(), None)?;
        reject_inapplicable_guard("kind", guards.kind.as_deref(), None)?;
        reject_inapplicable_guard("text_hash", guards.text_hash.as_deref(), None)?;
        reject_inapplicable_guard("fingerprint", guards.fingerprint.as_deref(), None)?;
    }

    Ok(ResolvedTarget::CoreProperties(ResolvedCoreProperties {
        part: resolved,
    }))
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
            .filter(|element| is_real_shape_tree_child(element))
            .nth(index)?;
    }
    Some(current)
}

fn element_text_hash(
    model: &PresentationDocument,
    slide_part: &PartName,
    element: &XmlElement,
    kind: ElementKind,
) -> Result<Option<String>> {
    if kind == ElementKind::GraphicFrameTable {
        let normalized = table_text_normalized(element);
        return Ok((!normalized.is_empty()).then(|| text_hash::text_hash(&normalized)));
    }
    if let Some(tx_body) = child_element(element, "txBody") {
        let text_body = read_text_body(tx_body);
        return Ok(Some(text_hash::text_hash(&text_body.normalized)));
    }
    if !matches!(
        kind,
        ElementKind::GraphicFrameChart | ElementKind::GraphicFrameDiagram
    ) {
        return Ok(None);
    }

    let Some(slide_rels) = model.package().relationships().set_for(slide_part) else {
        return Ok(None);
    };
    let mut rel_ids = Vec::new();
    collect_relationship_ids(element, &mut rel_ids);
    let mut normalized_parts = Vec::new();
    for rel_id in rel_ids {
        let Some(relationship) = slide_rels.get(&rel_id) else {
            continue;
        };
        if relationship.target_mode != TargetMode::Internal {
            continue;
        }
        let Some(part_name) = &relationship.resolved_target else {
            continue;
        };
        let Some(part) = model.package().parts().get(part_name) else {
            continue;
        };
        let document = parse_document(part.bytes()).map_err(|source| {
            Error::with_source(
                source.code(),
                format!("Could not parse related text part {part_name}."),
                source,
            )
        })?;
        let Some(root) = document.root_element() else {
            continue;
        };
        let normalized = related_text_normalized(root);
        if !normalized.is_empty() {
            normalized_parts.push(normalized);
        }
    }

    if normalized_parts.is_empty() {
        Ok(None)
    } else {
        Ok(Some(text_hash::text_hash(&normalized_parts.join("\n"))))
    }
}

fn table_text_normalized(element: &XmlElement) -> String {
    let Some(table) = first_descendant(element, "tbl") else {
        return String::new();
    };
    let normalized_parts = table
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .filter(|child| child.name.local_name == "tr")
        .flat_map(|row| {
            row.children
                .iter()
                .filter_map(XmlNode::as_element)
                .filter(|child| child.name.local_name == "tc")
        })
        .filter_map(|cell| child_element(cell, "txBody"))
        .map(read_text_body)
        .filter_map(|body| (!body.normalized.is_empty()).then_some(body.normalized))
        .collect::<Vec<_>>();
    normalized_parts.join("\n")
}

fn collect_relationship_ids(element: &XmlElement, output: &mut Vec<String>) {
    for attribute in &element.attributes {
        if matches!(
            attribute.name.prefix.as_deref(),
            Some("r") | Some("relationships")
        ) && !output.contains(&attribute.value)
        {
            output.push(attribute.value.clone());
        }
    }
    for child in element.children.iter().filter_map(XmlNode::as_element) {
        collect_relationship_ids(child, output);
    }
}

fn related_text_normalized(root: &XmlElement) -> String {
    let mut paragraphs = Vec::new();
    collect_related_paragraphs(root, &mut paragraphs);
    paragraphs.join("\n")
}

fn collect_related_paragraphs(element: &XmlElement, output: &mut Vec<String>) {
    if element.name.local_name == "p" {
        let tx_body = XmlElement {
            name: element.name.clone(),
            attributes: Vec::new(),
            namespaces: element.namespaces.clone(),
            children: vec![XmlNode::Element(element.clone())],
        };
        let body = read_text_body(&tx_body);
        if !body.normalized.is_empty() {
            output.push(body.normalized);
        }
        return;
    }
    for child in element.children.iter().filter_map(XmlNode::as_element) {
        collect_related_paragraphs(child, output);
    }
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

fn child_element<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a XmlElement> {
    element
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .find(|child| child.name.local_name == local_name)
}

fn exactly_one<T>(
    matches: Vec<T>,
    target_type: &str,
    selector: &str,
    element_id: Option<&str>,
    candidate_id: impl Fn(&T) -> String,
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
        let candidates = matches.iter().map(candidate_id).collect::<Vec<_>>();
        Err(selector_ambiguous(
            format!(
                "Selector for {target_type} {selector} resolved to {count} targets; expected exactly one. Candidates: {}.",
                candidates.join(", ")
            ),
            element_id,
            candidates,
        ))
    }
}

fn guard_eq(
    guard_name: &str,
    expected: Option<&str>,
    actual: &str,
    element_id: Option<&str>,
) -> Result<()> {
    if let Some(expected) = expected
        && expected != actual
    {
        return Err(selector_guard_failed(
            format!(
                "Selector guard {guard_name} did not match the current target: expected {expected}, actual {actual}."
            ),
            element_id,
            Some(expected),
            Some(actual),
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
        let actual = actual.unwrap_or("<none>");
        return Err(selector_guard_failed(
            format!(
                "Selector guard {guard_name} did not match the current target: expected {expected}, actual {actual}."
            ),
            element_id,
            Some(expected),
            Some(actual),
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
            Some("<not applicable>"),
            value,
        ));
    }
    Ok(())
}

fn selector_guard_failed(
    message: String,
    element_id: Option<&str>,
    expected: Option<&str>,
    actual: Option<&str>,
) -> Error {
    Error::new(ErrorCode::SelectorGuardFailed, message).with_location(ErrorLocation {
        element_id: element_id.map(str::to_owned),
        expected: expected.map(str::to_owned),
        actual: actual.map(str::to_owned),
        ..ErrorLocation::default()
    })
}

fn selector_not_found(message: String, element_id: Option<&str>) -> Error {
    Error::new(ErrorCode::SelectorNotFound, message).with_location(ErrorLocation {
        element_id: element_id.map(str::to_owned),
        ..ErrorLocation::default()
    })
}

fn selector_ambiguous(message: String, element_id: Option<&str>, candidates: Vec<String>) -> Error {
    Error::new(ErrorCode::SelectorAmbiguous, message).with_location(ErrorLocation {
        element_id: element_id.map(str::to_owned),
        candidates,
        ..ErrorLocation::default()
    })
}

fn guard_kind(
    guard_name: &str,
    expected: Option<&str>,
    kind: ElementKind,
    element_id: Option<&str>,
) -> Result<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    if kind_guard_matches(expected, kind) {
        return Ok(());
    }
    let actual = agent_view_kind_name(kind);
    Err(selector_guard_failed(
        format!(
            "Selector guard {guard_name} did not match the current target: expected {expected}, actual {actual}."
        ),
        element_id,
        Some(expected),
        Some(actual),
    ))
}

fn kind_guard_matches(expected: &str, kind: ElementKind) -> bool {
    expected == agent_view_kind_name(kind)
        || legacy_kind_name(kind).is_some_and(|legacy| expected == legacy)
}

const fn agent_view_kind_name(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::TextBox => "text_box",
        ElementKind::Shape => "shape",
        ElementKind::Picture => "image",
        ElementKind::Group => "group",
        ElementKind::GraphicFrameChart => "chart",
        ElementKind::GraphicFrameTable => "table",
        ElementKind::GraphicFrameDiagram => "diagram",
        ElementKind::GraphicFrameOle => "ole",
        ElementKind::GraphicFrameOther => "shape",
        ElementKind::Connector | ElementKind::Other => "shape",
    }
}

const fn legacy_kind_name(kind: ElementKind) -> Option<&'static str> {
    match kind {
        ElementKind::TextBox => None,
        ElementKind::Shape => None,
        ElementKind::Picture => None,
        ElementKind::Group => None,
        ElementKind::GraphicFrameChart
        | ElementKind::GraphicFrameTable
        | ElementKind::GraphicFrameDiagram
        | ElementKind::GraphicFrameOle
        | ElementKind::GraphicFrameOther => Some("graphic_frame"),
        ElementKind::Connector => Some("connector"),
        ElementKind::Other => Some("other"),
    }
}

#[cfg(test)]
use pptx_compose_core::{
    opc::{package::Package, relationships::Relationship},
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
            run: None,
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
                part_checksum: None,
            }),
            run: None,
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
            run: None,
        },
    )
    .expect_err("stale fingerprint fails");
    assert_eq!(stale.code(), ErrorCode::SelectorGuardFailed);
    assert!(stale.message().contains(
        "expected sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
    ));
    assert!(stale.message().contains("actual sha256:"));
    assert_eq!(
        stale.details().location.expected.as_deref(),
        Some("sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
    );
    assert_eq!(
        stale.details().location.actual.as_deref(),
        Some(element.fingerprint.as_str())
    );

    let ambiguous = resolve(
        &document,
        &Selector::MediaPart {
            part: None,
            guards: None,
        },
    )
    .expect_err("ambiguous media selector fails");
    assert_eq!(ambiguous.code(), ErrorCode::SelectorAmbiguous);
    assert_eq!(
        ambiguous.details().location.candidates,
        vec!["ppt/media/image1.png", "ppt/media/image2.png"]
    );
    assert!(ambiguous.message().contains("ppt/media/image1.png"));
}

#[cfg(test)]
#[test]
fn element_view_kind_guards_resolve_for_each_emitted_kind() {
    let document = fixture_document();
    for (id, guard_kind) in [
        ("slide-1:shape-4", "text_box"),
        ("slide-1:pic-5", "image"),
        ("slide-1:group-6", "group"),
        ("slide-1:graphic-7", "chart"),
        ("slide-1:graphic-11", "table"),
        ("slide-1:graphic-12", "diagram"),
        ("slide-1:graphic-13", "ole"),
        ("slide-1:graphic-14", "shape"),
        ("slide-1:shape-8", "shape"),
        ("slide-1:cxn-9", "shape"),
    ] {
        resolve(
            &document,
            &Selector::ElementId {
                id: id.to_owned(),
                guards: Some(Guards {
                    kind: Some(guard_kind.to_owned()),
                    ..Guards::default()
                }),
                run: None,
            },
        )
        .unwrap_or_else(|error| panic!("{id} should resolve with kind {guard_kind}: {error:?}"));
    }
}

#[cfg(test)]
#[test]
fn legacy_kind_guard_aliases_still_resolve() {
    let document = fixture_document();
    for (id, guard_kind) in [
        ("slide-1:graphic-7", "graphic_frame"),
        ("slide-1:graphic-11", "graphic_frame"),
        ("slide-1:graphic-12", "graphic_frame"),
        ("slide-1:graphic-13", "graphic_frame"),
        ("slide-1:graphic-14", "graphic_frame"),
        ("slide-1:cxn-9", "connector"),
    ] {
        resolve(
            &document,
            &Selector::ElementId {
                id: id.to_owned(),
                guards: Some(Guards {
                    kind: Some(guard_kind.to_owned()),
                    ..Guards::default()
                }),
                run: None,
            },
        )
        .unwrap_or_else(|error| {
            panic!("{id} should resolve with legacy kind {guard_kind}: {error:?}")
        });
    }
}

#[cfg(test)]
#[test]
fn resolves_core_properties_from_package_relationship() {
    use pptx_compose_core::provenance::checksum::part_checksum;

    let mut package = Package::new();
    insert(
        &mut package,
        "ppt/presentation.xml",
        presentation_xml_empty(),
    );
    insert(
        &mut package,
        "docProps/core.xml",
        br#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"/>"#,
    );
    package.push_relationship(Relationship::internal(
        RelationshipSource::Package,
        "rOffice",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
        "ppt/presentation.xml",
    ));
    package.push_relationship(Relationship::internal(
        RelationshipSource::Package,
        "rCore",
        CORE_PROPERTIES_REL_TYPE,
        "docProps/core.xml",
    ));

    let checksum = part_checksum(
        package
            .parts()
            .get(&part("docProps/core.xml"))
            .expect("core part exists")
            .bytes(),
    );
    let document = TestPresentationDocument::open(package).expect("presentation opens");
    let resolved = resolve(
        &document,
        &Selector::CoreProperties {
            part: "docProps/core.xml".to_owned(),
            guards: Some(Guards {
                part_checksum: Some(checksum),
                ..Guards::default()
            }),
        },
    )
    .expect("core properties resolves");

    assert!(matches!(
        resolved,
        ResolvedTarget::CoreProperties(target)
            if target.part.zip_entry_name() == "docProps/core.xml"
    ));

    let error = resolve(
        &document,
        &Selector::CoreProperties {
            part: "docProps/core.xml".to_owned(),
            guards: Some(Guards {
                part_checksum: Some(
                    "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                        .to_owned(),
                ),
                ..Guards::default()
            }),
        },
    )
    .expect_err("checksum mismatch fails");
    assert_eq!(error.code(), ErrorCode::SelectorGuardFailed);
}

#[cfg(test)]
#[test]
fn resolves_notes_slide_through_slide_relationship() {
    let mut package = fixture_package();
    let slide_part = part("ppt/slides/slide1.xml");
    insert(
        &mut package,
        "ppt/notesSlides/notesSlide1.xml",
        br#"<p:notes xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"/>"#,
    );
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(slide_part.clone()),
        "rNotes",
        NOTES_SLIDE_REL_TYPE,
        "../notesSlides/notesSlide1.xml",
    ));
    let document = TestPresentationDocument::open(package).expect("presentation opens");

    let resolved = resolve_notes_slide(
        &document,
        &Selector::SlideId {
            id: "slide-1".to_owned(),
            guards: None,
        },
    )
    .expect("notes slide resolves");

    assert!(matches!(
        resolved,
        ResolvedTarget::NotesSlide(target)
            if target.slide_id == "slide-1"
                && target.slide_part == slide_part
                && target.notes_part.zip_entry_name() == "ppt/notesSlides/notesSlide1.xml"
                && target.element_id == "slide-1:notes"
    ));

    let missing = resolve_notes_slide(
        &fixture_document(),
        &Selector::SlideId {
            id: "slide-1".to_owned(),
            guards: None,
        },
    )
    .expect_err("missing notes relationship is unsupported");
    assert_eq!(missing.code(), ErrorCode::UnsupportedEdit);
}

#[cfg(test)]
fn fixture_document() -> TestPresentationDocument {
    TestPresentationDocument::open(fixture_package()).expect("fixture presentation opens")
}

#[cfg(test)]
fn fixture_package() -> Package {
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

    package
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
fn presentation_xml_empty() -> &'static [u8] {
    br#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldIdLst/></p:presentation>"#
}

#[cfg(test)]
fn slide_xml() -> &'static [u8] {
    br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="4" name="Title 1"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:p><a:r><a:t>Quarterly Results</a:t></a:r></a:p></p:txBody></p:sp><p:pic><p:nvPicPr><p:cNvPr id="5" name="Picture 1"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill/><p:spPr/></p:pic><p:grpSp><p:nvGrpSpPr><p:cNvPr id="6" name="Group 1"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:grpSp><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="7" name="Chart 1"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"/></a:graphic></p:graphicFrame><p:sp><p:nvSpPr><p:cNvPr id="8" name="Shape 1"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr/></p:sp><p:cxnSp><p:nvCxnSpPr><p:cNvPr id="9" name="Connector 1"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr><p:spPr/></p:cxnSp><p:contentPart><p:nvContentPartPr><p:cNvPr id="10" name="Unknown 1"/></p:nvContentPartPr></p:contentPart><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="11" name="Table 1"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl/></a:graphicData></a:graphic></p:graphicFrame><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="12" name="Diagram 1"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"/></a:graphic></p:graphicFrame><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="13" name="OLE 1"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/presentationml/2006/ole"/></a:graphic></p:graphicFrame><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="14" name="Other Graphic 1"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="http://example.invalid/customGraphic"/></a:graphic></p:graphicFrame></p:spTree></p:cSld></p:sld>"#
}
