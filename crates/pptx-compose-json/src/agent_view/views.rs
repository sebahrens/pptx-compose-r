use std::collections::BTreeMap;

#[cfg(test)]
use pptx_compose_core::error::ErrorCode;
use pptx_compose_core::{
    error::Error,
    opc::{
        package::Package,
        part_name::PartName,
        relationships::{RelationshipSet, RelationshipSource},
    },
    pptx::presentation as core_presentation,
    pptx::{
        ids::{ElementKind as CoreElementKind, SpTreePath, agent_element_id},
        picture::read_picture,
        presentation::PresentationDocument,
        shape::{Shape, ShapeKind, read_shape},
        slide::Slide,
        text::read_text_body,
    },
    provenance::{
        checksum::part_checksum,
        cpj::{self, Cpj},
        document_id::document_id,
        fingerprint::{FingerprintInput, fingerprint},
        revision, text_hash,
    },
    validation::{ValidationMode, validate_package},
    xml::{
        document::{XmlElement, XmlNode},
        parser::parse_document,
    },
};
use pptx_compose_core::{
    opc::{content_types::ContentTypes, part::Part, relationships::resolve_internal_target},
    zip::reader::{RawEntry, from_bytes},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    AccessibilityView, AgentView, Bounds, Capabilities, Editable, EditableSupport, ElementKind,
    ElementPageView, ElementSelector, ElementView, FindTextResult, FindTextScope, ImageView,
    IntrinsicSizePx, Paragraph, PresentationView, Run, SelectorGuards, SlideView, StyleSummary,
    TextMatch, TextSpan, TextView, TruncationMarker, XmlLocation,
    pagination::{CursorScope, ViewMeta, bounded_limit, cursor_offset, paginate},
};
use crate::{
    schema_versions::{AGENT_VIEW_SCHEMA, AGENT_VIEW_VERSION, FIND_TEXT_SCHEMA, FIND_TEXT_VERSION},
    schemas::{
        FindingCategory, FindingCode, FindingView, JsonError, Severity, Summary, ValidationReport,
        ValidationStatus,
    },
};

pub type PptxPackage = PresentationDocument;

const TEXT_PREVIEW_CHARS: usize = 4_096;
const PARAGRAPH_PREVIEW_CHARS: usize = 1_024;
const RUN_PREVIEW_CHARS: usize = 1_024;
const ACCESSIBILITY_PREVIEW_CHARS: usize = 1_024;
const EMBEDDED_SLIDE_ELEMENT_LIMIT: u32 = 50;
const ALL_OP_NAMES: [&str; 9] = [
    "replace_text",
    "replace_notes_text",
    "replace_table_cell_text",
    "add_text_box",
    "move_resize_element",
    "set_alt_text",
    "set_document_metadata",
    "add_image",
    "replace_image",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    DeckSummary,
    SlidePage,
    SlideDetail,
    ElementDetail,
    MediaMetadata,
    ValidationReport,
}

impl ViewMode {
    #[must_use]
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::DeckSummary => "deck_summary",
            Self::SlidePage => "slide_page",
            Self::SlideDetail => "slide_detail",
            Self::ElementDetail => "element_detail",
            Self::MediaMetadata => "media_metadata",
            Self::ValidationReport => "validation_report",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewRequest {
    pub mode: ViewMode,
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_elements: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slide_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slide_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindTextRequest {
    pub query: String,
    pub scope: FindTextScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

pub fn build_view(pkg: &PptxPackage, req: ViewRequest) -> Result<Value, JsonError> {
    build_view_with_revision(pkg, revision::on_open().value(), req)
}

pub fn build_view_with_revision(
    pkg: &PptxPackage,
    revision: u64,
    req: ViewRequest,
) -> Result<Value, JsonError> {
    let context = ViewContext::new(pkg, revision)?;
    let mode = req.mode.as_token();
    let limit = bounded_limit(mode, req.limit)?;
    let scope = CursorScope {
        document_id: &context.document_id,
        revision: context.revision,
        mode,
        collection: None,
    };

    match req.mode {
        ViewMode::DeckSummary => Ok(to_value(context.agent_view(
            view_meta(mode, limit),
            0,
            Vec::new(),
            scope_value(&req),
            ViewPayload::default(),
        ))?),
        ViewMode::SlidePage => {
            let (slides, meta, omitted_count) = page_slide_summaries(
                &context,
                &req.slide_ids,
                req.include_elements,
                limit,
                req.cursor.as_deref(),
                scope,
            )?;
            Ok(to_value(context.agent_view(
                meta,
                omitted_count,
                slides,
                scope_value(&req),
                ViewPayload::default(),
            ))?)
        }
        ViewMode::SlideDetail => {
            let slide_id = req.slide_id.as_deref().ok_or_else(|| {
                JsonError::Projection("slide_detail requires slide_id.".to_owned())
            })?;
            let slide_ref = context
                .slide_ref(slide_id)
                .ok_or_else(|| JsonError::NotFound {
                    kind: "slide",
                    id: slide_id.to_owned(),
                })?;
            let mut media = BTreeMap::<String, ImageView>::new();
            let slide = project_slide(context.pkg, slide_ref, &mut media)?;
            let (detail, meta, omitted_count) = paginate_slide_elements(
                &slide,
                &context.document_id,
                context.revision,
                mode,
                slide_id,
                limit,
                req.cursor.as_deref(),
            )?;
            Ok(to_value(context.agent_view(
                meta,
                omitted_count,
                vec![detail],
                scope_value(&req),
                ViewPayload::default(),
            ))?)
        }
        ViewMode::ElementDetail => {
            let element_id = req.element_id.as_deref().ok_or_else(|| {
                JsonError::Projection("element_detail requires element_id.".to_owned())
            })?;
            let slide_id = element_id
                .split_once(':')
                .map(|(slide_id, _)| slide_id)
                .ok_or_else(|| {
                    JsonError::Projection(
                        "element_detail requires a slide-scoped element_id.".to_owned(),
                    )
                })?;
            let slide_ref = context
                .slide_ref(slide_id)
                .ok_or_else(|| JsonError::NotFound {
                    kind: "slide",
                    id: slide_id.to_owned(),
                })?;
            let mut media = BTreeMap::<String, ImageView>::new();
            let slide = project_slide(context.pkg, slide_ref, &mut media)?;
            let element = slide
                .detail
                .elements
                .iter()
                .find(|element| element.id == element_id)
                .ok_or_else(|| JsonError::NotFound {
                    kind: "element",
                    id: element_id.to_owned(),
                })?
                .clone();
            let mut detail = slide.summary.clone();
            detail.elements = vec![element];
            Ok(to_value(context.agent_view(
                view_meta(mode, limit),
                0,
                vec![detail],
                scope_value(&req),
                ViewPayload::default(),
            ))?)
        }
        ViewMode::MediaMetadata => {
            let (page, meta, omitted_count) =
                project_media_page(&context, limit, req.cursor.as_deref(), scope)?;
            Ok(to_value(context.agent_view(
                meta,
                omitted_count,
                Vec::new(),
                scope_value(&req),
                ViewPayload {
                    media: page,
                    ..ViewPayload::default()
                },
            ))?)
        }
        ViewMode::ValidationReport => {
            let validation = project_validation(
                context.pkg.package(),
                &context.document_id,
                context.revision,
            );
            let (page, meta, omitted_count) =
                paginate(&validation.findings, limit, req.cursor.as_deref(), scope)?;
            let findings = page.into_iter().cloned().collect();
            let mut validation = validation;
            validation.findings = findings;
            Ok(to_value(context.agent_view(
                meta,
                omitted_count,
                Vec::new(),
                scope_value(&req),
                ViewPayload {
                    validation: Some(validation),
                    ..ViewPayload::default()
                },
            ))?)
        }
    }
}

fn paginate_slide_elements(
    slide: &SlideProjection,
    document_id: &str,
    revision: u32,
    mode: &str,
    slide_id: &str,
    limit: u32,
    cursor: Option<&str>,
) -> Result<(SlideView, ViewMeta, u32), JsonError> {
    let detail_scope = CursorScope {
        document_id,
        revision,
        mode,
        collection: Some(slide_id),
    };
    let (elements, meta, omitted_count) =
        paginate(&slide.detail.elements, limit, cursor, detail_scope)?;
    let mut detail = slide.summary.clone();
    detail.elements = elements.into_iter().cloned().collect();
    Ok((detail, meta, omitted_count))
}

pub fn find_text(pkg: &PptxPackage, req: FindTextRequest) -> Result<FindTextResult, JsonError> {
    find_text_with_revision(pkg, revision::on_open().value(), req)
}

pub fn find_text_with_revision(
    pkg: &PptxPackage,
    revision: u64,
    req: FindTextRequest,
) -> Result<FindTextResult, JsonError> {
    if req.query.is_empty() {
        return Err(JsonError::Projection(
            "find_text query must not be empty.".to_owned(),
        ));
    }

    let context = ViewContext::new(pkg, revision)?;
    let mode = "find_text";
    let limit = bounded_limit(mode, req.limit)?;
    let scope = CursorScope {
        document_id: &context.document_id,
        revision: context.revision,
        mode,
        collection: None,
    };
    let start = cursor_offset(req.cursor.as_deref(), scope)?;
    let matches = collect_text_matches(&context, &req.query, &req.scope, start, limit)?;
    let (page, meta, omitted_count) = match_page(matches, limit, start, scope)?;

    Ok(FindTextResult {
        schema: FIND_TEXT_SCHEMA.to_owned(),
        version: FIND_TEXT_VERSION,
        document_id: context.document_id,
        revision: context.revision,
        query: req.query,
        scope: req.scope,
        view: meta,
        omitted_count,
        matches: page,
    })
}

#[derive(Clone)]
struct ViewContext<'a> {
    pkg: &'a PptxPackage,
    document_id: String,
    revision: u32,
    presentation_part: String,
    slide_count: u32,
    capabilities: Capabilities,
}

#[derive(Clone)]
struct SlideProjection {
    summary: SlideView,
    detail: SlideView,
}

#[derive(Default)]
struct ViewPayload {
    warnings: Vec<String>,
    media: Vec<ImageView>,
    validation: Option<ValidationReport>,
}

impl<'a> ViewContext<'a> {
    fn new(pkg: &'a PptxPackage, revision: u64) -> Result<Self, JsonError> {
        let document_id = package_document_id(pkg.package())?;
        let revision =
            u32::try_from(revision).map_err(|err| JsonError::Projection(err.to_string()))?;
        let slide_count = u32::try_from(pkg.slides().len())
            .map_err(|err| JsonError::Projection(err.to_string()))?;

        Ok(Self {
            pkg,
            document_id,
            revision,
            presentation_part: trim_part(pkg.presentation().part_name.as_str()),
            slide_count,
            capabilities: default_capabilities(),
        })
    }

    fn slide_ref(&self, slide_id: &str) -> Option<&Slide> {
        self.pkg
            .slides()
            .iter()
            .find(|slide| slide.agent_id() == slide_id)
    }

    fn agent_view(
        &self,
        view: ViewMeta,
        omitted_count: u32,
        slides: Vec<SlideView>,
        scope: Cpj,
        payload: ViewPayload,
    ) -> AgentView {
        AgentView {
            schema: AGENT_VIEW_SCHEMA.to_owned(),
            version: AGENT_VIEW_VERSION,
            document_id: self.document_id.clone(),
            revision: self.revision,
            view_id: local_view_id(&self.document_id, self.revision, &view.mode, &scope),
            view,
            omitted_count,
            capabilities: self.capabilities.clone(),
            presentation: PresentationView {
                part: self.presentation_part.clone(),
                slide_count: self.slide_count,
            },
            slides,
            warnings: payload.warnings,
            media: payload.media,
            validation: payload.validation,
        }
    }
}

fn collect_text_matches(
    context: &ViewContext,
    query: &str,
    scope: &FindTextScope,
    start: u32,
    limit: u32,
) -> Result<Vec<TextMatch>, JsonError> {
    let mut matches = Vec::new();
    let mut seen = 0_u32;
    let stop_after = start.saturating_add(limit).saturating_add(1);
    for slide_ref in context.pkg.slides() {
        let mut media = BTreeMap::<String, ImageView>::new();
        let slide = project_slide(context.pkg, slide_ref, &mut media)?;
        if let FindTextScope::Slide { slide_id } = scope
            && slide.detail.id != *slide_id
        {
            continue;
        }
        for element in &slide.detail.elements {
            let Some(text) = &element.text else {
                continue;
            };
            for span in find_query_spans(&text.plain, query)? {
                if seen < start {
                    seen = seen.saturating_add(1);
                    continue;
                }
                if seen >= stop_after {
                    return Ok(matches);
                }
                matches.push(TextMatch {
                    slide_id: slide.detail.id.clone(),
                    slide_index: slide.detail.index,
                    element_id: element.id.clone(),
                    kind: element.kind,
                    part: element.part.clone(),
                    fingerprint: element.fingerprint.clone(),
                    text_hash: text.text_hash.clone(),
                    span,
                    matched_text: substring_by_char_span(&text.plain, span),
                    selector: ElementSelector {
                        selector_type: "element_id".to_owned(),
                        id: element.id.clone(),
                        guards: SelectorGuards {
                            slide_id: slide.detail.id.clone(),
                            kind: element.kind,
                            part: element.part.clone(),
                            text_hash: text.text_hash.clone(),
                            fingerprint: element.fingerprint.clone(),
                        },
                    },
                });
                seen = seen.saturating_add(1);
            }
        }
    }

    if let FindTextScope::Slide { slide_id } = scope
        && context.slide_ref(slide_id).is_none()
    {
        return Err(JsonError::NotFound {
            kind: "slide",
            id: slide_id.clone(),
        });
    }

    Ok(matches)
}

fn match_page(
    mut matches: Vec<TextMatch>,
    limit: u32,
    start: u32,
    scope: CursorScope<'_>,
) -> Result<(Vec<TextMatch>, ViewMeta, u32), JsonError> {
    let limit_usize =
        usize::try_from(limit).map_err(|err| JsonError::InvalidCursor(err.to_string()))?;
    let truncated = matches.len() > limit_usize;
    if truncated {
        matches.truncate(limit_usize);
    }
    let next_cursor = if truncated {
        Some(super::pagination::Cursor::encode(
            start.saturating_add(limit),
            scope,
        )?)
    } else {
        None
    };
    Ok((
        matches,
        ViewMeta {
            mode: scope.mode.to_owned(),
            limit,
            next_cursor,
            truncated,
        },
        u32::from(truncated),
    ))
}

fn find_query_spans(text: &str, query: &str) -> Result<Vec<TextSpan>, JsonError> {
    let mut spans = Vec::new();
    let mut search_start = 0;
    while let Some(relative_start) = text[search_start..].find(query) {
        let byte_start = search_start + relative_start;
        let byte_end = byte_start + query.len();
        spans.push(TextSpan {
            start: u32::try_from(text[..byte_start].chars().count())
                .map_err(|err| JsonError::Projection(err.to_string()))?,
            end: u32::try_from(text[..byte_end].chars().count())
                .map_err(|err| JsonError::Projection(err.to_string()))?,
        });
        search_start = byte_end;
    }
    Ok(spans)
}

fn substring_by_char_span(text: &str, span: TextSpan) -> String {
    text.chars()
        .skip(span.start as usize)
        .take(span.end.saturating_sub(span.start) as usize)
        .collect()
}

fn page_slide_summaries(
    context: &ViewContext,
    slide_ids: &[String],
    include_elements: bool,
    limit: u32,
    cursor: Option<&str>,
    scope: CursorScope<'_>,
) -> Result<(Vec<SlideView>, ViewMeta, u32), JsonError> {
    let slide_refs = scoped_slide_refs(context, slide_ids)?;
    let (page, meta, omitted_count) = paginate(&slide_refs, limit, cursor, scope)?;
    let slides = if include_elements {
        let mut media = BTreeMap::<String, ImageView>::new();
        page.into_iter()
            .map(|slide| {
                let slide = project_slide(context.pkg, slide, &mut media)?;
                cap_embedded_slide_elements(&slide, &context.document_id, context.revision)
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        page.into_iter()
            .map(|slide| project_slide_summary(context.pkg, slide))
            .collect::<Result<Vec<_>, _>>()?
    };
    Ok((slides, meta, omitted_count))
}

fn project_media_page(
    context: &ViewContext,
    limit: u32,
    cursor: Option<&str>,
    scope: CursorScope<'_>,
) -> Result<(Vec<ImageView>, ViewMeta, u32), JsonError> {
    let start = cursor_offset(cursor, scope)?;
    let stop_after = start.saturating_add(limit).saturating_add(1);
    let mut media = BTreeMap::<String, ImageView>::new();

    for slide in context.pkg.slides() {
        if u32::try_from(media.len()).unwrap_or(u32::MAX) >= stop_after {
            break;
        }
        project_slide(context.pkg, slide, &mut media)?;
    }

    let mut media = media.into_values().collect::<Vec<_>>();
    let start_usize =
        usize::try_from(start).map_err(|err| JsonError::InvalidCursor(err.to_string()))?;
    if start_usize > media.len() {
        return Err(JsonError::InvalidCursor(
            "Cursor offset is outside the requested collection.".to_owned(),
        ));
    }
    let limit_usize =
        usize::try_from(limit).map_err(|err| JsonError::InvalidCursor(err.to_string()))?;
    let truncated = media.len().saturating_sub(start_usize) > limit_usize;
    if truncated {
        media.truncate(start_usize.saturating_add(limit_usize));
    }
    let page = media[start_usize..].to_vec();
    let next_cursor = if truncated {
        Some(super::pagination::Cursor::encode(
            start.saturating_add(limit),
            scope,
        )?)
    } else {
        None
    };

    Ok((
        page,
        ViewMeta {
            mode: scope.mode.to_owned(),
            limit,
            next_cursor,
            truncated,
        },
        u32::from(truncated),
    ))
}

fn cap_embedded_slide_elements(
    slide: &SlideProjection,
    document_id: &str,
    revision: u32,
) -> Result<SlideView, JsonError> {
    let slide_id = slide.summary.id.as_str();
    let scope = CursorScope {
        document_id,
        revision,
        mode: ViewMode::SlideDetail.as_token(),
        collection: Some(slide_id),
    };
    let (elements, meta, omitted_count) = paginate(
        &slide.detail.elements,
        EMBEDDED_SLIDE_ELEMENT_LIMIT,
        None,
        scope,
    )?;
    let mut detail = slide.summary.clone();
    detail.elements = elements.into_iter().cloned().collect();
    if meta.truncated {
        detail.elements_page = Some(ElementPageView {
            mode: meta.mode,
            limit: meta.limit,
            next_cursor: meta.next_cursor,
            truncated: meta.truncated,
            omitted_count,
            detail: format!(
                "Request slide_detail for {slide_id} and pass next_cursor to continue elements."
            ),
        });
    }
    Ok(detail)
}

fn project_slide(
    pkg: &PptxPackage,
    slide: &Slide,
    media: &mut BTreeMap<String, ImageView>,
) -> Result<SlideProjection, JsonError> {
    let part = pkg.package().parts().get(&slide.part_name).ok_or_else(|| {
        JsonError::Projection(format!("Slide part {} is missing.", slide.part_name))
    })?;
    let document = parse_document(part.bytes()).map_err(core_error)?;
    let root = document.root_element().ok_or_else(|| {
        JsonError::Projection(format!(
            "Slide part {} has no root element.",
            slide.part_name
        ))
    })?;
    let sp_tree = first_descendant(root, "spTree").ok_or_else(|| {
        JsonError::Projection(format!("Slide part {} has no p:spTree.", slide.part_name))
    })?;
    let rels = pkg
        .package()
        .relationships()
        .set_for(&slide.part_name)
        .cloned()
        .unwrap_or_else(|| RelationshipSet {
            source: slide.part_name.clone(),
            rels: Vec::new(),
        });
    let mut elements = Vec::new();
    collect_elements(
        sp_tree,
        &slide.agent_id(),
        slide.part_name.clone(),
        &rels,
        pkg.package(),
        &mut elements,
        media,
    )?;
    let summary = project_slide_summary(pkg, slide)?;
    let mut detail = summary.clone();
    detail.elements = elements;
    Ok(SlideProjection { summary, detail })
}

fn project_slide_summary(pkg: &PptxPackage, slide: &Slide) -> Result<SlideView, JsonError> {
    let part = pkg.package().parts().get(&slide.part_name).ok_or_else(|| {
        JsonError::Projection(format!("Slide part {} is missing.", slide.part_name))
    })?;
    Ok(SlideView {
        id: slide.agent_id(),
        index: slide.agent_index,
        ppt_slide_id: Some(slide.id.value()),
        part: trim_part(slide.part_name.as_str()),
        relationship_id: presentation_relationship_id(pkg, &slide.part_name),
        layout_part: slide
            .layout
            .as_ref()
            .map(|layout| trim_part(layout.part_name.as_str()))
            .unwrap_or_default(),
        part_checksum: part_checksum(part.bytes()),
        elements: Vec::new(),
        elements_page: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn collect_elements(
    parent: &XmlElement,
    slide_id: &str,
    slide_part: PartName,
    slide_rels: &RelationshipSet,
    package: &Package,
    output: &mut Vec<ElementView>,
    media: &mut BTreeMap<String, ImageView>,
) -> Result<(), JsonError> {
    collect_elements_at(
        parent,
        slide_id,
        slide_part,
        slide_rels,
        package,
        &[],
        &[],
        output,
        media,
    )
}

#[allow(clippy::too_many_arguments)]
fn collect_elements_at(
    parent: &XmlElement,
    slide_id: &str,
    slide_part: PartName,
    slide_rels: &RelationshipSet,
    package: &Package,
    path_prefix: &[u32],
    group_path: &[u32],
    output: &mut Vec<ElementView>,
    media: &mut BTreeMap<String, ImageView>,
) -> Result<(), JsonError> {
    for (zero_based_index, child) in parent
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .filter(|element| is_drawable_shape_tree_child(element))
        .enumerate()
    {
        let child_index = u32::try_from(zero_based_index + 1)
            .map_err(|err| JsonError::Projection(err.to_string()))?;
        let mut sp_tree_path = path_prefix.to_vec();
        sp_tree_path.push(child_index);
        let path = SpTreePath {
            sp_tree_path: sp_tree_path.clone(),
            group_path: group_path.to_vec(),
        };
        let core_kind = core_element_kind(child);
        let element = project_element(
            child,
            slide_id,
            slide_part.clone(),
            path.clone(),
            core_kind,
            slide_rels,
            package,
            media,
        )?;
        output.push(element);

        if core_kind == CoreElementKind::Group {
            collect_elements_at(
                child,
                slide_id,
                slide_part.clone(),
                slide_rels,
                package,
                &sp_tree_path,
                &sp_tree_path,
                output,
                media,
            )?;
        }
    }
    Ok(())
}

fn is_drawable_shape_tree_child(element: &XmlElement) -> bool {
    matches!(
        element.name.local_name.as_str(),
        "sp" | "pic" | "graphicFrame" | "grpSp" | "cxnSp"
    )
}

#[allow(clippy::too_many_arguments)]
fn project_element(
    element: &XmlElement,
    slide_id: &str,
    slide_part: PartName,
    path: SpTreePath,
    core_kind: CoreElementKind,
    slide_rels: &RelationshipSet,
    package: &Package,
    media: &mut BTreeMap<String, ImageView>,
) -> Result<ElementView, JsonError> {
    let shape = read_shape(element, path.clone());
    let text = first_descendant(element, "txBody").map(project_text);
    let text_hash = text.as_ref().map(|text| text.text_hash.clone());
    let mut image_support = if core_kind == CoreElementKind::Picture {
        ImageEditSupport::Unresolved
    } else {
        ImageEditSupport::NotPicture
    };
    let image = if core_kind == CoreElementKind::Picture {
        match read_picture(element, path.clone(), slide_rels, package) {
            Ok(picture) if !picture.external => {
                image_support = ImageEditSupport::Embedded;
                let Some(media_part) = picture.media_part else {
                    return Err(JsonError::Projection(
                        "Embedded picture did not resolve to a media part.".to_owned(),
                    ));
                };
                let image = ImageView {
                    relationship_id: picture.embed_rel_id,
                    media_part: trim_part(media_part.as_str()),
                    content_type: picture.content_type,
                    byte_length: picture.byte_length,
                    checksum: package
                        .parts()
                        .get(&media_part)
                        .map(|part| part_checksum(part.bytes()))
                        .unwrap_or_else(|| "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_owned()),
                    intrinsic_size_px: picture.intrinsic_size_px.map(|size| IntrinsicSizePx {
                        width: size.width,
                        height: size.height,
                    }),
                    shared_media_ref_count: picture.shared_media_ref_count,
                };
                media
                    .entry(image.media_part.clone())
                    .or_insert_with(|| image.clone());
                Some(image)
            }
            Ok(_) => {
                image_support = ImageEditSupport::ExternalLink;
                None
            }
            Err(_) => None,
        }
    } else {
        None
    };

    Ok(ElementView {
        id: agent_element_id(slide_id, core_kind, shape.cnvpr_id, &path),
        kind: json_element_kind(core_kind),
        slide_id: slide_id.to_owned(),
        part: trim_part(slide_part.as_str()),
        xml_location: xml_location(element, &shape, &path),
        z_order: path.sp_tree_path.last().copied().unwrap_or(0),
        bounds: shape.bounds.as_ref().map_or(
            Bounds {
                x: 0,
                y: 0,
                cx: 0,
                cy: 0,
            },
            |bounds| Bounds {
                x: bounds.x,
                y: bounds.y,
                cx: bounds.cx,
                cy: bounds.cy,
            },
        ),
        editable: editable(
            text.is_some() && core_kind.supports_replace_text(),
            image_support,
        ),
        fingerprint: fingerprint(&FingerprintInput {
            kind: core_kind,
            part: slide_part,
            sp_tree_path: path.sp_tree_path,
            group_path: path.group_path,
            cnvpr_id: shape.cnvpr_id,
            text_hash,
        }),
        accessibility: project_accessibility(&shape),
        text,
        image,
    })
}

fn project_accessibility(shape: &Shape) -> Option<AccessibilityView> {
    let title = shape.alt_text_title.as_ref().map(|title| {
        truncate_text(
            title.clone(),
            ACCESSIBILITY_PREVIEW_CHARS,
            "Use element_detail for this element to inspect the full accessibility title.",
        )
        .0
    });
    let description = shape.alt_text_description.as_ref().map(|description| {
        truncate_text(
            description.clone(),
            ACCESSIBILITY_PREVIEW_CHARS,
            "Use element_detail for this element to inspect the full accessibility description.",
        )
        .0
    });
    if title.is_none() && description.is_none() {
        None
    } else {
        Some(AccessibilityView { title, description })
    }
}

fn project_text(tx_body: &XmlElement) -> TextView {
    let body = read_text_body(tx_body);
    let text_hash = text_hash::text_hash(&body.normalized);
    let plain = truncate_text(
        body.plain,
        TEXT_PREVIEW_CHARS,
        "Use element_detail for this element to inspect text context.",
    );
    let normalized = truncate_text(
        body.normalized,
        TEXT_PREVIEW_CHARS,
        "Use element_detail for this element to inspect normalized text context.",
    );
    let truncation = merge_truncation(&plain.1, &normalized.1);
    let paragraphs = body
        .paragraphs
        .into_iter()
        .map(|paragraph| {
            let paragraph_text = truncate_text(
                paragraph.runs.iter().map(|run| run.text.as_str()).collect(),
                PARAGRAPH_PREVIEW_CHARS,
                "Use element_detail for this element to inspect the full paragraph text.",
            );
            Paragraph {
                text: paragraph_text.0,
                runs: paragraph
                    .runs
                    .into_iter()
                    .map(|run| {
                        let run_text = truncate_text(
                            run.text,
                            RUN_PREVIEW_CHARS,
                            "Use element_detail for this element to inspect the full run text.",
                        );
                        Run {
                            text: run_text.0,
                            style_summary: StyleSummary {
                                font_size_pt: run.style_summary.font_size_pt,
                                bold: run.style_summary.bold,
                                italic: run.style_summary.italic,
                                underline: run.style_summary.underline,
                                font_color_rgb: run.style_summary.font_color_rgb,
                                latin_typeface: run.style_summary.latin_typeface,
                                language: run.style_summary.language,
                            },
                            truncation: run_text.1,
                        }
                    })
                    .collect(),
                truncation: paragraph_text.1,
            }
        })
        .collect();
    TextView {
        plain: plain.0,
        normalized: normalized.0,
        paragraphs,
        text_hash,
        truncation,
    }
}

fn truncate_text(
    text: String,
    limit: usize,
    detail: &'static str,
) -> (String, Option<TruncationMarker>) {
    let original_chars = text.chars().count();
    if original_chars <= limit {
        return (text, None);
    }

    let truncated = text.chars().take(limit).collect::<String>();
    (
        truncated,
        Some(TruncationMarker {
            original_chars: u32::try_from(original_chars).unwrap_or(u32::MAX),
            shown_chars: u32::try_from(limit).unwrap_or(u32::MAX),
            detail: detail.to_owned(),
        }),
    )
}

fn merge_truncation(
    plain: &Option<TruncationMarker>,
    normalized: &Option<TruncationMarker>,
) -> Option<TruncationMarker> {
    plain.clone().or_else(|| normalized.clone())
}

fn project_validation(package: &Package, document_id: &str, revision: u32) -> ValidationReport {
    let outcome = validate_package(package, ValidationMode::NoEdit);
    ValidationReport {
        schema: crate::schema_versions::VALIDATION_REPORT_SCHEMA.to_owned(),
        version: crate::schema_versions::VALIDATION_REPORT_VERSION,
        document_id: document_id.to_owned(),
        revision,
        status: match outcome.status {
            pptx_compose_core::validation::ValidationStatus::Valid => ValidationStatus::Valid,
            pptx_compose_core::validation::ValidationStatus::Invalid => ValidationStatus::Invalid,
        },
        summary: Summary {
            fatal: u32::try_from(outcome.summary.fatal).unwrap_or(u32::MAX),
            errors: u32::try_from(outcome.summary.errors).unwrap_or(u32::MAX),
            warnings: u32::try_from(outcome.summary.warnings).unwrap_or(u32::MAX),
            info: u32::try_from(outcome.summary.info).unwrap_or(u32::MAX),
        },
        findings: outcome
            .findings
            .into_iter()
            .map(|finding| FindingView {
                id: finding.id,
                severity: match finding.severity {
                    pptx_compose_core::validation::Severity::Info => Severity::Info,
                    pptx_compose_core::validation::Severity::Warning => Severity::Warning,
                    pptx_compose_core::validation::Severity::Error => Severity::Error,
                    pptx_compose_core::validation::Severity::Fatal => Severity::Fatal,
                },
                category: match finding.category {
                    pptx_compose_core::validation::FindingCategory::ContentType => {
                        FindingCategory::ContentType
                    }
                    pptx_compose_core::validation::FindingCategory::Relationship => {
                        FindingCategory::Relationship
                    }
                    pptx_compose_core::validation::FindingCategory::Presentation => {
                        FindingCategory::Presentation
                    }
                    pptx_compose_core::validation::FindingCategory::Slide => FindingCategory::Slide,
                    pptx_compose_core::validation::FindingCategory::Comments => {
                        FindingCategory::Comments
                    }
                    pptx_compose_core::validation::FindingCategory::Xml => FindingCategory::Xml,
                    pptx_compose_core::validation::FindingCategory::Package => {
                        FindingCategory::Package
                    }
                    pptx_compose_core::validation::FindingCategory::Signature => {
                        FindingCategory::Signature
                    }
                },
                code: match finding.code {
                    pptx_compose_core::validation::FindingCode::MissingContentType => {
                        FindingCode::MissingContentType
                    }
                    pptx_compose_core::validation::FindingCode::MediaContentTypeMismatch => {
                        FindingCode::MediaContentTypeMismatch
                    }
                    pptx_compose_core::validation::FindingCode::DanglingInternalRelationship => {
                        FindingCode::DanglingInternalRelationship
                    }
                    pptx_compose_core::validation::FindingCode::UnreferencedMedia => {
                        FindingCode::UnreferencedMedia
                    }
                    pptx_compose_core::validation::FindingCode::UnresolvedRelationshipReference => {
                        FindingCode::UnresolvedRelationshipReference
                    }
                    pptx_compose_core::validation::FindingCode::DuplicateRelationshipId => {
                        FindingCode::DuplicateRelationshipId
                    }
                    pptx_compose_core::validation::FindingCode::ExternalRelationshipNotChecked => {
                        FindingCode::ExternalRelationshipNotChecked
                    }
                    pptx_compose_core::validation::FindingCode::DanglingCommentAuthorRef => {
                        FindingCode::DanglingCommentAuthorRef
                    }
                    pptx_compose_core::validation::FindingCode::DuplicateSlideId => {
                        FindingCode::DuplicateSlideId
                    }
                    pptx_compose_core::validation::FindingCode::SlideOrderMismatch => {
                        FindingCode::SlideOrderMismatch
                    }
                    pptx_compose_core::validation::FindingCode::DuplicateDrawingId => {
                        FindingCode::DuplicateDrawingId
                    }
                    pptx_compose_core::validation::FindingCode::InvalidBounds => {
                        FindingCode::InvalidBounds
                    }
                    pptx_compose_core::validation::FindingCode::MalformedXml => {
                        FindingCode::MalformedXml
                    }
                    pptx_compose_core::validation::FindingCode::MissingNamespaceDeclaration => {
                        FindingCode::MissingNamespaceDeclaration
                    }
                    pptx_compose_core::validation::FindingCode::PartDropped => {
                        FindingCode::PartDropped
                    }
                    pptx_compose_core::validation::FindingCode::OrphanPart => {
                        FindingCode::OrphanPart
                    }
                    pptx_compose_core::validation::FindingCode::SignatureInvalidatedByEdit => {
                        FindingCode::SignatureInvalidatedByEdit
                    }
                },
                message: finding.message,
                blocking: finding.blocking,
                location: finding.location,
                suggested_action: finding.suggested_action,
            })
            .collect(),
    }
}

fn package_document_id(package: &Package) -> Result<String, JsonError> {
    let content_types_name = PartName::from_zip_entry("[Content_Types].xml").map_err(core_error)?;
    let content_types = package.parts().get(&content_types_name).ok_or_else(|| {
        JsonError::Projection("Package is missing [Content_Types].xml.".to_owned())
    })?;
    let parts = package
        .parts()
        .iter()
        .filter(|part| part.name() != &content_types_name)
        .map(|part| (part.name().clone(), part.bytes()))
        .collect::<Vec<_>>();
    Ok(document_id(&parts, content_types.bytes()))
}

fn local_view_id(document_id: &str, revision: u32, mode: &str, scope: &Cpj) -> String {
    let mut preimage = BTreeMap::new();
    preimage.insert("document_id".to_owned(), Cpj::Str(document_id.to_owned()));
    preimage.insert("mode".to_owned(), Cpj::Str(mode.to_owned()));
    preimage.insert("revision".to_owned(), Cpj::Uint(u64::from(revision)));
    preimage.insert(
        "schema".to_owned(),
        Cpj::Str("pptx-compose.view_id.v1".to_owned()),
    );
    preimage.insert("scope".to_owned(), scope.clone());
    cpj::digest_cpj(&Cpj::Object(preimage))
}

fn scope_value(req: &ViewRequest) -> Cpj {
    let mut scope = BTreeMap::new();
    if let Some(slide_id) = &req.slide_id {
        scope.insert("slide_id".to_owned(), Cpj::Str(slide_id.clone()));
    }
    if !req.slide_ids.is_empty() {
        scope.insert(
            "slide_ids".to_owned(),
            Cpj::Array(
                req.slide_ids
                    .iter()
                    .map(|slide_id| Cpj::Str(slide_id.clone()))
                    .collect(),
            ),
        );
    }
    if let Some(element_id) = &req.element_id {
        scope.insert("element_id".to_owned(), Cpj::Str(element_id.clone()));
    }
    if let Some(cursor) = &req.cursor {
        scope.insert("cursor".to_owned(), Cpj::Str(cursor.clone()));
    }
    if let Some(limit) = req.limit {
        scope.insert("limit".to_owned(), Cpj::Uint(u64::from(limit)));
    }
    Cpj::Object(scope)
}

fn scoped_slide_refs<'a>(
    context: &'a ViewContext<'a>,
    slide_ids: &[String],
) -> Result<Vec<&'a Slide>, JsonError> {
    if slide_ids.is_empty() {
        return Ok(context.pkg.slides().iter().collect());
    }

    let mut slides = Vec::with_capacity(slide_ids.len());
    for slide_id in slide_ids {
        let slide = context
            .slide_ref(slide_id)
            .ok_or_else(|| JsonError::NotFound {
                kind: "slide",
                id: slide_id.clone(),
            })?;
        slides.push(slide);
    }
    Ok(slides)
}

fn view_meta(mode: &str, limit: u32) -> ViewMeta {
    ViewMeta {
        mode: mode.to_owned(),
        limit,
        next_cursor: None,
        truncated: false,
    }
}

fn default_capabilities() -> Capabilities {
    Capabilities {
        operations: ALL_OP_NAMES.iter().map(|op| (*op).to_owned()).collect(),
        media_content_types: vec![
            "image/png".to_owned(),
            "image/jpeg".to_owned(),
            "image/gif".to_owned(),
        ],
        units: "emu".to_owned(),
    }
}

#[derive(Clone, Copy)]
enum ImageEditSupport {
    Embedded,
    ExternalLink,
    NotPicture,
    Unresolved,
}

fn editable(has_text: bool, image_support: ImageEditSupport) -> Editable {
    let (image_supported, image_reason) = match image_support {
        ImageEditSupport::Embedded => (true, None),
        ImageEditSupport::ExternalLink => (false, Some("external_link".to_owned())),
        ImageEditSupport::NotPicture => (false, Some("not_picture".to_owned())),
        ImageEditSupport::Unresolved => (false, None),
    };

    Editable {
        text: EditableSupport {
            supported: has_text,
            reason: (!has_text).then(|| "not_text".to_owned()),
        },
        bounds: EditableSupport {
            supported: true,
            reason: None,
        },
        image: EditableSupport {
            supported: image_supported,
            reason: image_reason,
        },
    }
}

fn xml_location(element: &XmlElement, shape: &Shape, path: &SpTreePath) -> XmlLocation {
    XmlLocation {
        sp_tree_path: path.sp_tree_path.clone(),
        group_path: path.group_path.clone(),
        element_tag: element.name.raw.clone(),
        cnvpr_id: shape
            .cnvpr_id
            .and_then(|id| u32::try_from(id).ok())
            .unwrap_or(0),
        cnvpr_name: shape.name.clone().unwrap_or_default(),
    }
}

fn core_element_kind(element: &XmlElement) -> CoreElementKind {
    match read_shape(
        element,
        SpTreePath {
            sp_tree_path: Vec::new(),
            group_path: Vec::new(),
        },
    )
    .kind
    {
        ShapeKind::TextBox => CoreElementKind::TextBox,
        ShapeKind::AutoShape => CoreElementKind::Shape,
        ShapeKind::Picture => CoreElementKind::Picture,
        ShapeKind::GraphicFrameChart => CoreElementKind::GraphicFrameChart,
        ShapeKind::GraphicFrameTable => CoreElementKind::GraphicFrameTable,
        ShapeKind::GraphicFrameDiagram => CoreElementKind::GraphicFrameDiagram,
        ShapeKind::GraphicFrameOle => CoreElementKind::GraphicFrameOle,
        ShapeKind::GraphicFrameOther => CoreElementKind::GraphicFrameOther,
        ShapeKind::Group => CoreElementKind::Group,
        ShapeKind::Connector => CoreElementKind::Connector,
        ShapeKind::Other => CoreElementKind::Other,
    }
}

const fn json_element_kind(kind: CoreElementKind) -> ElementKind {
    match kind {
        CoreElementKind::TextBox => ElementKind::TextBox,
        CoreElementKind::Picture => ElementKind::Image,
        CoreElementKind::Group => ElementKind::Group,
        CoreElementKind::GraphicFrameChart => ElementKind::Chart,
        CoreElementKind::GraphicFrameTable => ElementKind::Table,
        CoreElementKind::GraphicFrameDiagram => ElementKind::Diagram,
        CoreElementKind::GraphicFrameOle => ElementKind::Ole,
        CoreElementKind::Shape
        | CoreElementKind::GraphicFrameOther
        | CoreElementKind::Connector
        | CoreElementKind::Other => ElementKind::Shape,
    }
}

fn presentation_relationship_id(pkg: &PptxPackage, slide_part: &PartName) -> String {
    pkg.package()
        .relationships()
        .iter()
        .find(|relationship| {
            relationship.source == RelationshipSource::Part(pkg.presentation().part_name.clone())
                && relationship
                    .resolved_target
                    .as_ref()
                    .is_some_and(|target| target == slide_part)
        })
        .map(|relationship| relationship.id.clone())
        .unwrap_or_default()
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

fn to_value<T: Serialize>(value: T) -> Result<Value, JsonError> {
    serde_json::to_value(value).map_err(|err| JsonError::SerializeSchema(err.to_string()))
}

fn trim_part(part_name: &str) -> String {
    part_name.trim_start_matches('/').to_owned()
}

const fn is_false(value: &bool) -> bool {
    !*value
}

fn core_error(error: Error) -> JsonError {
    JsonError::Core(error)
}

pub fn package_from_pptx_bytes(bytes: &[u8]) -> Result<PptxPackage, JsonError> {
    let entries = from_bytes(bytes).map_err(core_error)?;
    let package = package_from_entries(&entries)?;
    PresentationDocument::open(package).map_err(core_error)
}

fn package_from_entries(entries: &[RawEntry]) -> Result<Package, JsonError> {
    let mut package = Package::new();
    for entry in entries {
        package
            .insert_part(
                Part::from_zip_entry(entry.meta.original_name.clone(), entry.bytes.clone())
                    .map_err(core_error)?,
            )
            .map_err(core_error)?;
    }
    hydrate_content_types(&mut package)?;
    hydrate_relationships(&mut package)?;
    core_presentation::hydrate_package_slide_ids(&mut package);
    Ok(package)
}

fn hydrate_content_types(package: &mut Package) -> Result<(), JsonError> {
    let content_types_name = PartName::from_zip_entry("[Content_Types].xml").map_err(core_error)?;
    let raw = package
        .parts()
        .get(&content_types_name)
        .ok_or_else(|| JsonError::Projection("Package is missing [Content_Types].xml.".to_owned()))?
        .bytes();
    *package.content_types_mut() = ContentTypes::parse(raw).map_err(core_error)?;
    Ok(())
}

fn hydrate_relationships(package: &mut Package) -> Result<(), JsonError> {
    let rels_entries = package
        .parts()
        .iter()
        .filter(|part| part.name().as_str().ends_with(".rels"))
        .map(|part| (part.name().clone(), part.bytes().to_vec()))
        .collect::<Vec<_>>();

    for (rels_part_name, raw) in rels_entries {
        if rels_part_name.as_str() == "/_rels/.rels" {
            let mut set = RelationshipSet::parse(&rels_part_name, &raw).map_err(core_error)?;
            for mut relationship in set.rels.drain(..) {
                relationship.source = RelationshipSource::Package;
                relationship.resolved_target = Some(
                    resolve_internal_target(&RelationshipSource::Package, &relationship.target)
                        .map_err(core_error)?,
                );
                package.push_relationship(relationship);
            }
            continue;
        }

        let source = relationship_source_for(&rels_part_name)?;
        let set = RelationshipSet::parse(&source, &raw).map_err(core_error)?;
        for relationship in set.rels {
            package.push_relationship(relationship);
        }
    }
    Ok(())
}

fn relationship_source_for(rels_part_name: &PartName) -> Result<PartName, JsonError> {
    let path = rels_part_name.as_str();
    let Some((directory, file_name)) = path.rsplit_once("/_rels/") else {
        return Err(JsonError::Projection(format!(
            "Relationship part {rels_part_name} is not in a _rels directory."
        )));
    };
    let Some(source_file) = file_name.strip_suffix(".rels") else {
        return Err(JsonError::Projection(format!(
            "Relationship part {rels_part_name} does not end with .rels."
        )));
    };
    PartName::from_zip_entry(format!("{directory}/{source_file}").as_str()).map_err(core_error)
}

#[cfg(test)]
#[test]
fn all_modes() {
    let pkg = package_from_pptx_bytes(include_bytes!(
        "../../../../fixtures/real-world/worldbank-cpf-concept-note.pptx"
    ))
    .expect("fixture package parses");
    let slide_count = pkg.slides().len();

    let deck_summary = build_view(
        &pkg,
        ViewRequest {
            mode: ViewMode::DeckSummary,
            include_elements: false,
            slide_id: None,
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: None,
        },
    )
    .expect("deck_summary builds");
    assert_eq!(deck_summary["schema"], AGENT_VIEW_SCHEMA);
    assert_eq!(deck_summary["version"], 1);
    assert_eq!(deck_summary["presentation"]["slide_count"], slide_count);
    assert_eq!(
        deck_summary["slides"]
            .as_array()
            .expect("slides array")
            .len(),
        0
    );
    assert_eq!(deck_summary.pointer("/slides/0/elements"), None);

    let slide_detail = build_view(&pkg, request_for(&pkg, ViewMode::SlideDetail, None))
        .expect("slide_detail builds");
    let element_id = slide_detail["slides"][0]["elements"][0]["id"]
        .as_str()
        .expect("fixture exposes at least one generic element")
        .to_owned();
    assert_eq!(slide_detail["schema"], AGENT_VIEW_SCHEMA);
    assert_eq!(slide_detail["version"], 1);
    assert_eq!(slide_detail["view"]["mode"], "slide_detail");

    for mode in [
        ViewMode::SlidePage,
        ViewMode::ElementDetail,
        ViewMode::MediaMetadata,
        ViewMode::ValidationReport,
    ] {
        let req = request_for(&pkg, mode, Some(element_id.clone()));
        let value = build_view(&pkg, req).expect("view mode builds");
        assert_eq!(value["schema"], AGENT_VIEW_SCHEMA);
        assert_eq!(value["version"], 1);
        assert_eq!(value["view"]["mode"], mode.as_token());
    }

    let err = build_view(
        &pkg,
        ViewRequest {
            mode: ViewMode::ElementDetail,
            include_elements: false,
            slide_id: None,
            slide_ids: Vec::new(),
            element_id: Some("slide-1:missing-999".to_owned()),
            cursor: None,
            limit: None,
        },
    )
    .expect_err("missing element returns structured error");
    assert!(matches!(
        err,
        JsonError::NotFound {
            kind: "element",
            ..
        }
    ));
}

#[cfg(test)]
#[test]
fn image_editability_matches_embedded_and_external_picture_support() {
    use pptx_compose_core::{
        opc::{
            package::{OFFICE_DOCUMENT_REL_TYPE, Package},
            part_name::PartName,
            relationships::{Relationship, RelationshipSource},
        },
        pptx::{media::IMAGE_REL_TYPE, presentation::PresentationDocument},
    };

    const SLIDE_REL_TYPE: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";

    let presentation_part = PartName::from_zip_entry("ppt/presentation.xml").expect("part name");
    let slide_part = PartName::from_zip_entry("ppt/slides/slide1.xml").expect("part name");
    let media_part = PartName::from_zip_entry("ppt/media/image1.png").expect("part name");

    let mut package = Package::new();
    package
        .insert_zip_entry("[Content_Types].xml", content_types_xml().to_vec())
        .expect("content types part inserts");
    package
        .insert_zip_entry("ppt/presentation.xml", presentation_xml().to_vec())
        .expect("presentation part inserts");
    package
        .insert_zip_entry("ppt/slides/slide1.xml", picture_slide_xml().to_vec())
        .expect("slide part inserts");
    package
        .insert_zip_entry("ppt/media/image1.png", b"embedded image bytes".to_vec())
        .expect("media part inserts");
    package
        .content_types_mut()
        .insert_default("png", "image/png");

    package.push_relationship(Relationship::internal(
        RelationshipSource::Package,
        "rOffice",
        OFFICE_DOCUMENT_REL_TYPE,
        "ppt/presentation.xml",
    ));
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(presentation_part),
        "rSlide",
        SLIDE_REL_TYPE,
        "slides/slide1.xml",
    ));
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(slide_part.clone()),
        "rEmbed",
        IMAGE_REL_TYPE,
        "../media/image1.png",
    ));
    package.push_relationship(Relationship::external(
        RelationshipSource::Part(slide_part),
        "rLink",
        IMAGE_REL_TYPE,
        "https://example.test/image.png",
    ));

    let pkg = PresentationDocument::open(package).expect("presentation opens");
    let value = build_view(&pkg, request_for(&pkg, ViewMode::SlideDetail, None))
        .expect("slide detail builds");
    let elements = value["slides"][0]["elements"]
        .as_array()
        .expect("elements array");

    assert_eq!(elements.len(), 2);
    assert!(elements.iter().all(|element| {
        let tag = &element["xml_location"]["element_tag"];
        tag != "p:nvGrpSpPr" && tag != "p:grpSpPr"
    }));

    let embedded = elements
        .iter()
        .find(|element| element["xml_location"]["cnvpr_name"] == "Embedded Picture")
        .expect("embedded picture is projected");
    assert_eq!(embedded["editable"]["image"]["supported"], true);
    assert_eq!(embedded["editable"]["image"].get("reason"), None);
    assert_eq!(embedded["image"]["relationship_id"], "rEmbed");

    let linked = elements
        .iter()
        .find(|element| element["xml_location"]["cnvpr_name"] == "Linked Picture")
        .expect("linked picture is projected");
    assert_eq!(linked["editable"]["image"]["supported"], false);
    assert_eq!(linked["editable"]["image"]["reason"], "external_link");
    assert_eq!(linked.get("image"), None);

    assert_eq!(
        pkg.package()
            .parts()
            .get(&media_part)
            .expect("embedded media part remains present")
            .bytes(),
        b"embedded image bytes"
    );
}

#[cfg(test)]
#[test]
fn graphic_frame_kinds_are_emitted_from_graphic_data_uri() {
    use pptx_compose_core::{
        opc::{
            package::{OFFICE_DOCUMENT_REL_TYPE, Package},
            part_name::PartName,
            relationships::{Relationship, RelationshipSource},
        },
        pptx::presentation::PresentationDocument,
    };

    const SLIDE_REL_TYPE: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";

    let presentation_part = PartName::from_zip_entry("ppt/presentation.xml").expect("part name");

    let mut package = Package::new();
    package
        .insert_zip_entry("[Content_Types].xml", content_types_xml().to_vec())
        .expect("content types part inserts");
    package
        .insert_zip_entry("ppt/presentation.xml", presentation_xml().to_vec())
        .expect("presentation part inserts");
    package
        .insert_zip_entry("ppt/slides/slide1.xml", graphic_frame_slide_xml().to_vec())
        .expect("slide part inserts");

    package.push_relationship(Relationship::internal(
        RelationshipSource::Package,
        "rOffice",
        OFFICE_DOCUMENT_REL_TYPE,
        "ppt/presentation.xml",
    ));
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(presentation_part),
        "rSlide",
        SLIDE_REL_TYPE,
        "slides/slide1.xml",
    ));

    let pkg = PresentationDocument::open(package).expect("presentation opens");
    let value = build_view(&pkg, request_for(&pkg, ViewMode::SlideDetail, None))
        .expect("slide detail builds");
    let elements = value["slides"][0]["elements"]
        .as_array()
        .expect("elements array");

    for (name, kind) in [
        ("Chart Frame", "chart"),
        ("Table Frame", "table"),
        ("Diagram Frame", "diagram"),
        ("OLE Frame", "ole"),
        ("Unknown Frame", "shape"),
    ] {
        let element = elements
            .iter()
            .find(|element| element["xml_location"]["cnvpr_name"] == name)
            .unwrap_or_else(|| panic!("{name} should be projected"));
        assert_eq!(element["kind"], kind);
    }
}

#[cfg(test)]
#[test]
fn slide_detail_paginates_elements_with_working_cursor() {
    let slide = slide_projection_with_elements(3);
    let document_id = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    let (first_page, meta, omitted_count) = paginate_slide_elements(
        &slide,
        document_id,
        1,
        ViewMode::SlideDetail.as_token(),
        "slide-1",
        2,
        None,
    )
    .expect("first element page builds");

    assert_eq!(first_page.elements.len(), 2);
    assert_eq!(first_page.elements[0].id, "slide-1:shape-0");
    assert_eq!(first_page.elements[1].id, "slide-1:shape-1");
    assert_eq!(meta.mode, "slide_detail");
    assert_eq!(meta.limit, 2);
    assert!(meta.truncated);
    assert_eq!(omitted_count, 1);

    let next_cursor = meta.next_cursor.expect("first page has a cursor");
    let (second_page, meta, omitted_count) = paginate_slide_elements(
        &slide,
        document_id,
        1,
        ViewMode::SlideDetail.as_token(),
        "slide-1",
        2,
        Some(&next_cursor),
    )
    .expect("cursor retrieves second element page");

    assert_eq!(second_page.elements.len(), 1);
    assert_eq!(second_page.elements[0].id, "slide-1:shape-2");
    assert!(!meta.truncated);
    assert_eq!(meta.next_cursor, None);
    assert_eq!(omitted_count, 0);

    let err = paginate_slide_elements(
        &slide,
        document_id,
        1,
        ViewMode::SlideDetail.as_token(),
        "slide-2",
        2,
        Some(&next_cursor),
    )
    .expect_err("element cursor is scoped to its slide");
    assert!(matches!(err, JsonError::InvalidCursor(_)));
}

#[cfg(test)]
#[test]
fn slide_page_embedded_elements_are_capped_with_marker() {
    let slide = slide_projection_with_elements(EMBEDDED_SLIDE_ELEMENT_LIMIT + 3);
    let document_id = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    let page_slide =
        cap_embedded_slide_elements(&slide, document_id, 1).expect("embedded elements are capped");

    assert_eq!(
        page_slide.elements.len(),
        EMBEDDED_SLIDE_ELEMENT_LIMIT as usize
    );
    let elements_page = page_slide
        .elements_page
        .expect("capped element list has pagination marker");
    assert_eq!(elements_page.mode, "slide_detail");
    assert_eq!(elements_page.limit, EMBEDDED_SLIDE_ELEMENT_LIMIT);
    assert!(elements_page.truncated);
    assert_eq!(elements_page.omitted_count, 3);
    assert!(elements_page.next_cursor.is_some());
    assert!(elements_page.detail.contains("slide_detail"));
    assert!(elements_page.detail.contains("slide-1"));
}

#[cfg(test)]
fn presentation_xml() -> &'static [u8] {
    br#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="256" r:id="rSlide"/></p:sldIdLst></p:presentation>"#
}

#[cfg(test)]
fn content_types_xml() -> &'static [u8] {
    br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="png" ContentType="image/png"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/></Types>"#
}

#[cfg(test)]
fn picture_slide_xml() -> &'static [u8] {
    br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:pic><p:nvPicPr><p:cNvPr id="5" name="Embedded Picture"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rEmbed"/></p:blipFill><p:spPr/></p:pic><p:pic><p:nvPicPr><p:cNvPr id="6" name="Linked Picture"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:link="rLink"/></p:blipFill><p:spPr/></p:pic></p:spTree></p:cSld></p:sld>"#
}

#[cfg(test)]
fn graphic_frame_slide_xml() -> &'static [u8] {
    br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="7" name="Chart Frame"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"/></a:graphic></p:graphicFrame><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="8" name="Table Frame"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl/></a:graphicData></a:graphic></p:graphicFrame><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="9" name="Diagram Frame"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"/></a:graphic></p:graphicFrame><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="10" name="OLE Frame"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/presentationml/2006/ole"/></a:graphic></p:graphicFrame><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="11" name="Unknown Frame"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="http://example.invalid/customGraphic"/></a:graphic></p:graphicFrame></p:spTree></p:cSld></p:sld>"#
}

#[cfg(test)]
#[test]
fn summary_views_do_not_parse_slide_xml() {
    let mut entries = from_bytes(include_bytes!("../../../../fixtures/minimal.pptx"))
        .expect("fixture zip parses");
    let slide = entries
        .iter_mut()
        .find(|entry| entry.name.as_str() == "/ppt/slides/slide1.xml")
        .expect("fixture has slide1");
    slide.bytes = b"<not-xml".to_vec();
    let package = package_from_entries(&entries).expect("package hydrates without slide parsing");
    let pkg = PresentationDocument::open(package).expect("presentation opens");

    let deck_summary = build_view(
        &pkg,
        ViewRequest {
            mode: ViewMode::DeckSummary,
            include_elements: false,
            slide_id: None,
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: None,
        },
    )
    .expect("deck summary does not parse slide XML");
    assert_eq!(deck_summary["presentation"]["slide_count"], 1);

    let slide_page = build_view(
        &pkg,
        ViewRequest {
            mode: ViewMode::SlidePage,
            include_elements: false,
            slide_id: None,
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: Some(1),
        },
    )
    .expect("slide page summary does not parse slide XML");
    assert_eq!(slide_page["slides"].as_array().expect("slides").len(), 1);

    let err = build_view(&pkg, request_for(&pkg, ViewMode::SlideDetail, None))
        .expect_err("slide detail still parses its target slide");
    assert!(matches!(
        err,
        JsonError::Core(error) if error.code() != ErrorCode::InvalidInput
    ));
}

#[cfg(test)]
#[test]
fn long_text_payloads_are_truncated_with_markers() {
    let long = "x".repeat(TEXT_PREVIEW_CHARS + 50);
    let xml = format!("<txBody><p><r><t>{long}</t></r></p></txBody>");
    let document = parse_document(xml.as_bytes()).expect("text body XML parses");
    let text = project_text(document.root_element().expect("txBody root"));

    assert_eq!(text.plain.chars().count(), TEXT_PREVIEW_CHARS);
    let truncation = text.truncation.expect("text truncation marker");
    assert_eq!(truncation.original_chars, (TEXT_PREVIEW_CHARS + 50) as u32);
    assert_eq!(truncation.shown_chars, TEXT_PREVIEW_CHARS as u32);
    assert_eq!(
        text.paragraphs[0]
            .truncation
            .as_ref()
            .expect("paragraph truncation")
            .shown_chars,
        PARAGRAPH_PREVIEW_CHARS as u32
    );
    assert_eq!(
        text.paragraphs[0].runs[0]
            .truncation
            .as_ref()
            .expect("run truncation")
            .shown_chars,
        RUN_PREVIEW_CHARS as u32
    );
}

#[cfg(test)]
fn slide_projection_with_elements(count: u32) -> SlideProjection {
    let summary = SlideView {
        id: "slide-1".to_owned(),
        index: 0,
        ppt_slide_id: Some(256),
        part: "ppt/slides/slide1.xml".to_owned(),
        relationship_id: "rId1".to_owned(),
        layout_part: "ppt/slideLayouts/slideLayout1.xml".to_owned(),
        part_checksum: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .to_owned(),
        elements: Vec::new(),
        elements_page: None,
    };
    let mut detail = summary.clone();
    detail.elements = (0..count).map(test_element).collect();
    SlideProjection { summary, detail }
}

#[cfg(test)]
fn test_element(index: u32) -> ElementView {
    ElementView {
        id: format!("slide-1:shape-{index}"),
        kind: ElementKind::Shape,
        slide_id: "slide-1".to_owned(),
        part: "ppt/slides/slide1.xml".to_owned(),
        xml_location: XmlLocation {
            sp_tree_path: vec![index],
            group_path: Vec::new(),
            element_tag: "p:sp".to_owned(),
            cnvpr_id: index + 1,
            cnvpr_name: format!("Shape {index}"),
        },
        z_order: index,
        bounds: Bounds {
            x: 0,
            y: 0,
            cx: 1000,
            cy: 1000,
        },
        editable: Editable {
            text: EditableSupport {
                supported: false,
                reason: Some("shape has no text body".to_owned()),
            },
            bounds: EditableSupport {
                supported: true,
                reason: None,
            },
            image: EditableSupport {
                supported: false,
                reason: Some("shape is not an image".to_owned()),
            },
        },
        fingerprint: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .to_owned(),
        accessibility: None,
        text: None,
        image: None,
    }
}

#[cfg(test)]
fn request_for(
    pkg: &pptx_compose_core::pptx::presentation::PresentationDocument,
    mode: ViewMode,
    element_id: Option<String>,
) -> ViewRequest {
    let slide_id = pkg
        .slides()
        .first()
        .map(pptx_compose_core::pptx::slide::Slide::agent_id);
    ViewRequest {
        mode,
        include_elements: false,
        slide_id: matches!(mode, ViewMode::SlideDetail)
            .then(|| slide_id.clone())
            .flatten(),
        slide_ids: Vec::new(),
        element_id: matches!(mode, ViewMode::ElementDetail)
            .then(|| element_id.clone())
            .flatten(),
        cursor: None,
        limit: None,
    }
}
