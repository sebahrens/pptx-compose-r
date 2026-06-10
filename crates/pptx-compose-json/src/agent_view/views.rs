use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::BTreeSet;

#[cfg(test)]
use pptx_compose_core::error::ErrorCode;
use pptx_compose_core::{
    error::Error,
    opc::{
        package::Package,
        part_name::PartName,
        relationships::{RelationshipSet, RelationshipSource, TargetMode},
    },
    pptx::presentation as core_presentation,
    pptx::{
        ids::{ElementKind as CoreElementKind, SpTreePath, agent_element_id},
        picture::read_picture,
        presentation::PresentationDocument,
        shape::{Shape, ShapeKind, read_shape},
        slide::Slide,
        text::{merge_text_bodies, project_element_text, read_text_body},
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
    AccessibilityView, AgentView, AutofitKind, BodyPrView, Bounds, Capabilities, Editable,
    EditableSupport, ElementKind, ElementPageView, ElementSelector, ElementView, FindTextResult,
    FindTextRunSelector, FindTextScope, ImageView, IntrinsicSizePx, Paragraph,
    ParagraphDefaultsView, PlaceholderView, PresentationView, Run, SelectorGuards, SlideView,
    StyleConfidence, StyleSummary, TableCell, TableCellRef, TableRow, TableView,
    TextCoverageWarning, TextLayoutView, TextMatch, TextSpan, TextView, TruncationMarker,
    XmlLocation,
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
const NOTES_SLIDE_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide";
const ALL_OP_NAMES: [&str; 7] = [
    "replace_text",
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
    let cursor_collection = find_text_cursor_collection(&req.query, &req.scope);
    let scope = CursorScope {
        document_id: &context.document_id,
        revision: context.revision,
        mode,
        collection: Some(&cursor_collection),
    };
    let start = cursor_offset(req.cursor.as_deref(), scope)?;
    let matches = collect_text_matches(&context, &req.query, &req.scope, start, limit)?;
    let warnings = collect_text_coverage_warnings(&context, &req.scope)?;
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
        warnings,
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

fn find_text_cursor_collection(query: &str, scope: &FindTextScope) -> String {
    let scope_key = match scope {
        FindTextScope::Deck => "deck".to_owned(),
        FindTextScope::Slide { slide_id } => format!("slide:{slide_id}"),
    };
    format!("query:{}:{query}\0scope:{scope_key}", query.len())
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
            collect_element_text_matches(
                element,
                &slide,
                query,
                start,
                stop_after,
                &mut seen,
                &mut matches,
            )?;
            if seen >= stop_after {
                return Ok(matches);
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

fn collect_text_coverage_warnings(
    context: &ViewContext,
    scope: &FindTextScope,
) -> Result<Vec<TextCoverageWarning>, JsonError> {
    let mut warnings = Vec::new();
    for slide_ref in context.pkg.slides() {
        let mut media = BTreeMap::<String, ImageView>::new();
        let slide = project_slide(context.pkg, slide_ref, &mut media)?;
        if let FindTextScope::Slide { slide_id } = scope
            && slide.detail.id != *slide_id
        {
            continue;
        }
        warnings.extend(
            slide
                .detail
                .elements
                .iter()
                .flat_map(|element| element.text_coverage_warnings.iter().cloned()),
        );
    }

    if let FindTextScope::Slide { slide_id } = scope
        && context.slide_ref(slide_id).is_none()
    {
        return Err(JsonError::NotFound {
            kind: "slide",
            id: slide_id.clone(),
        });
    }

    Ok(warnings)
}

fn collect_element_text_matches(
    element: &ElementView,
    slide: &SlideProjection,
    query: &str,
    start: u32,
    stop_after: u32,
    seen: &mut u32,
    matches: &mut Vec<TextMatch>,
) -> Result<(), JsonError> {
    if let Some(table) = &element.table {
        for row in &table.rows {
            for cell in &row.cells {
                let Some(text) = &cell.text else {
                    continue;
                };
                if !cell
                    .editable
                    .text
                    .as_ref()
                    .is_some_and(|support| support.supported)
                {
                    continue;
                }
                collect_text_view_matches(
                    element,
                    slide,
                    text,
                    Some(TableCellRef {
                        row: cell.row,
                        col: cell.col,
                    }),
                    query,
                    start,
                    stop_after,
                    seen,
                    matches,
                )?;
                if *seen >= stop_after {
                    return Ok(());
                }
            }
        }
        return Ok(());
    }
    if let Some(text) = &element.text
        && element
            .editable
            .text
            .as_ref()
            .is_none_or(|support| support.supported)
    {
        collect_text_view_matches(
            element, slide, text, None, query, start, stop_after, seen, matches,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn collect_text_view_matches(
    element: &ElementView,
    slide: &SlideProjection,
    text: &TextView,
    cell: Option<TableCellRef>,
    query: &str,
    start: u32,
    stop_after: u32,
    seen: &mut u32,
    matches: &mut Vec<TextMatch>,
) -> Result<(), JsonError> {
    let guard_text_hash = element
        .text
        .as_ref()
        .map_or_else(|| text.text_hash.clone(), |text| text.text_hash.clone());
    for span in find_query_spans(search_plain_text(text), query)? {
        let run = run_selector_for_span(text, span);
        if related_text_requires_run_selector(element.kind) && run.is_none() {
            continue;
        }
        if *seen < start {
            *seen = seen.saturating_add(1);
            continue;
        }
        if *seen >= stop_after {
            return Ok(());
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
            matched_text: substring_by_char_span(search_plain_text(text), span),
            selector: ElementSelector {
                selector_type: if is_notes_element(element) {
                    "slide_id".to_owned()
                } else {
                    "element_id".to_owned()
                },
                id: if is_notes_element(element) {
                    slide.detail.id.clone()
                } else {
                    element.id.clone()
                },
                guards: SelectorGuards {
                    slide_id: slide.detail.id.clone(),
                    kind: element.kind,
                    part: element.part.clone(),
                    text_hash: guard_text_hash.clone(),
                    fingerprint: element.fingerprint.clone(),
                },
                run,
            },
            cell,
        });
        *seen = seen.saturating_add(1);
    }
    Ok(())
}

fn is_notes_element(element: &ElementView) -> bool {
    element.id == format!("{}:notes", element.slide_id)
}

fn related_text_requires_run_selector(kind: ElementKind) -> bool {
    matches!(kind, ElementKind::Chart | ElementKind::Diagram)
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

fn search_plain_text(text: &TextView) -> &str {
    if text.full_plain.is_empty() {
        &text.plain
    } else {
        &text.full_plain
    }
}

fn search_paragraph_text(paragraph: &Paragraph) -> &str {
    if paragraph.full_text.is_empty() {
        &paragraph.text
    } else {
        &paragraph.full_text
    }
}

fn search_run_text(run: &Run) -> &str {
    if run.full_text.is_empty() {
        &run.text
    } else {
        &run.full_text
    }
}

fn run_selector_for_span(text: &TextView, span: TextSpan) -> Option<FindTextRunSelector> {
    if span.start >= span.end {
        return None;
    }

    let mut paragraph_start = 0_u32;
    for (paragraph_offset, paragraph) in text.paragraphs.iter().enumerate() {
        let paragraph_index = u32::try_from(paragraph_offset).ok()?;
        let paragraph_text = search_paragraph_text(paragraph);
        let mut run_ranges = Vec::new();
        let mut search_start = 0_usize;
        for (run_index, run) in paragraph.runs.iter().enumerate() {
            let run_index = u32::try_from(run_index).ok()?;
            let run_text = search_run_text(run);
            let relative_start = paragraph_text[search_start..].find(run_text)?;
            let byte_start = search_start + relative_start;
            let byte_end = byte_start + run_text.len();
            let run_start = paragraph_start
                .checked_add(u32::try_from(paragraph_text[..byte_start].chars().count()).ok()?)?;
            let run_end = paragraph_start
                .checked_add(u32::try_from(paragraph_text[..byte_end].chars().count()).ok()?)?;
            run_ranges.push((run_index, run_start, run_end, run_text));
            search_start = byte_end;
        }
        let paragraph_end =
            paragraph_start.checked_add(u32::try_from(paragraph_text.chars().count()).ok()?)?;
        if span.start >= paragraph_start && span.end <= paragraph_end {
            let start = run_ranges
                .iter()
                .find(|(_, run_start, _, _)| *run_start == span.start)?;
            let end = run_ranges
                .iter()
                .find(|(_, _, run_end, _)| *run_end == span.end)?;
            if start.0 > end.0 {
                return None;
            }
            let selected = substring_by_char_span(
                paragraph_text,
                TextSpan {
                    start: span.start.saturating_sub(paragraph_start),
                    end: span.end.saturating_sub(paragraph_start),
                },
            );
            return Some(FindTextRunSelector {
                paragraph_index,
                run_index: start.0,
                run_end_index: (end.0 > start.0).then_some(end.0),
                text_hash: Some(text_hash::text_hash(&selected)),
            });
        }
        paragraph_start = if paragraph_offset + 1 < text.paragraphs.len() {
            paragraph_end.checked_add(1)?
        } else {
            paragraph_end
        };
    }
    None
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
    let mut media = BTreeMap::<String, ImageView>::new();

    for slide in context.pkg.slides() {
        project_slide(context.pkg, slide, &mut media)?;
    }

    let media = media.into_values().collect::<Vec<_>>();
    let (page, meta, omitted_count) = paginate(&media, limit, cursor, scope)?;
    Ok((page.into_iter().cloned().collect(), meta, omitted_count))
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
    let layout_part = slide
        .layout
        .as_ref()
        .map(|layout| trim_part(layout.part_name.as_str()));
    collect_elements(
        sp_tree,
        &slide.agent_id(),
        slide.part_name.clone(),
        layout_part.as_deref(),
        &rels,
        pkg.package(),
        &mut elements,
        media,
    )?;
    if let Some(notes) = project_notes_element(pkg, slide, &rels)? {
        elements.push(notes);
    }
    let summary = project_slide_summary(pkg, slide)?;
    let mut detail = summary.clone();
    detail.elements = elements;
    Ok(SlideProjection { summary, detail })
}

fn project_notes_element(
    pkg: &PptxPackage,
    slide: &Slide,
    slide_rels: &RelationshipSet,
) -> Result<Option<ElementView>, JsonError> {
    let Some(notes_part) = resolve_notes_part(slide_rels)? else {
        return Ok(None);
    };
    let Some(part) = pkg.package().parts().get(&notes_part) else {
        return Ok(None);
    };
    let document = parse_document(part.bytes()).map_err(core_error)?;
    let root = document.root_element().ok_or_else(|| {
        JsonError::Projection(format!(
            "Speaker-notes part {notes_part} has no root element."
        ))
    })?;
    let Some((shape_element, tx_body)) = notes_body_shape_tx_body(root) else {
        return Ok(None);
    };

    let slide_id = slide.agent_id();
    let element_id = format!("{slide_id}:notes");
    let text = project_text_body(read_text_body(tx_body));
    let shape = read_shape(
        shape_element,
        SpTreePath {
            sp_tree_path: Vec::new(),
            group_path: Vec::new(),
        },
    );
    let part = trim_part(notes_part.as_str());
    let fingerprint = fingerprint(&FingerprintInput {
        kind: CoreElementKind::Shape,
        part: notes_part.clone(),
        sp_tree_path: Vec::new(),
        group_path: Vec::new(),
        cnvpr_id: shape.cnvpr_id,
        text_hash: Some(text.text_hash.clone()),
    });

    Ok(Some(ElementView {
        id: element_id,
        kind: ElementKind::Shape,
        slide_id,
        part,
        xml_location: XmlLocation {
            sp_tree_path: Vec::new(),
            group_path: Vec::new(),
            element_tag: shape_element.name.raw.clone(),
            cnvpr_id: shape
                .cnvpr_id
                .and_then(|id| u32::try_from(id).ok())
                .unwrap_or(0),
            cnvpr_name: shape.name.unwrap_or_default(),
        },
        z_order: 0,
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
        editable: Editable {
            text: Some(EditableSupport {
                supported: true,
                reason: None,
            }),
            bounds: None,
            alt_text: None,
            image: None,
        },
        fingerprint,
        accessibility: None,
        placeholder: project_placeholder(shape_element, None),
        text_layout: None,
        text: Some(text),
        table: None,
        image: None,
        text_coverage_warnings: Vec::new(),
    }))
}

fn resolve_notes_part(slide_rels: &RelationshipSet) -> Result<Option<PartName>, JsonError> {
    let Some(relationship) = slide_rels
        .rels
        .iter()
        .find(|relationship| relationship.rel_type == NOTES_SLIDE_REL_TYPE)
    else {
        return Ok(None);
    };
    if relationship.target_mode != TargetMode::Internal {
        return Ok(None);
    }
    relationship
        .resolved_target
        .clone()
        .map_or_else(
            || resolve_internal_target(&relationship.source, &relationship.target),
            Ok,
        )
        .map(Some)
        .map_err(core_error)
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
    layout_part: Option<&str>,
    slide_rels: &RelationshipSet,
    package: &Package,
    output: &mut Vec<ElementView>,
    media: &mut BTreeMap<String, ImageView>,
) -> Result<(), JsonError> {
    collect_elements_at(
        parent,
        slide_id,
        slide_part,
        layout_part,
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
    layout_part: Option<&str>,
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
            layout_part,
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
                layout_part,
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
    layout_part: Option<&str>,
    path: SpTreePath,
    core_kind: CoreElementKind,
    slide_rels: &RelationshipSet,
    package: &Package,
    media: &mut BTreeMap<String, ImageView>,
) -> Result<ElementView, JsonError> {
    let shape = read_shape(element, path.clone());
    let (table, table_text) = if core_kind == CoreElementKind::GraphicFrameTable {
        project_table(element)
    } else {
        (None, None)
    };
    let text_projection = if table_text.is_some() {
        TextProjection {
            text: table_text,
            support: None,
            uneditable_related_text_reason: None,
        }
    } else {
        projected_element_text(element, core_kind, slide_rels, package)?
    };
    let text = text_projection.text;
    let text_hash = text.as_ref().map(|text| text.text_hash.clone());
    let mut image_support = ImageEditSupport::UnresolvedPicture;
    let image = if core_kind == CoreElementKind::Picture {
        match read_picture(element, path.clone(), slide_rels, package) {
            Ok(picture) if !picture.external && picture.link_rel_id.is_none() => {
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
            Err(_) => {
                image_support = unresolved_picture_support(element, slide_rels);
                None
            }
        }
    } else {
        None
    };

    let id = agent_element_id(slide_id, core_kind, shape.cnvpr_id, &path);
    let kind = json_element_kind(core_kind);
    let part = trim_part(slide_part.as_str());
    let mut text_coverage_warnings = text_coverage_warnings_for_element(
        slide_id,
        &id,
        kind,
        &part,
        core_kind,
        shape.bounds.as_ref(),
    );
    if let Some(reason) = text_projection.uneditable_related_text_reason.as_deref() {
        text_coverage_warnings.push(chart_text_coverage_warning(
            slide_id, &id, kind, &part, reason,
        ));
    }
    if let Some(reason) = text_projection
        .support
        .as_ref()
        .filter(|support| !support.supported)
        .and_then(|support| support.reason.as_deref())
    {
        text_coverage_warnings.push(TextCoverageWarning {
            code: "diagram_text_unsupported".to_owned(),
            slide_id: slide_id.to_owned(),
            element_id: id.clone(),
            kind,
            part: part.clone(),
            reason: reason.to_owned(),
            detail: diagram_text_coverage_warning_detail(reason).to_owned(),
        });
    }

    Ok(ElementView {
        id,
        kind,
        slide_id: slide_id.to_owned(),
        part: part.clone(),
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
            core_kind,
            text.is_some(),
            shape.cnvpr_id.is_some(),
            image_support,
            text_projection.support,
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
        placeholder: project_placeholder(element, layout_part),
        text_layout: project_text_layout(element),
        text,
        table,
        image,
        text_coverage_warnings,
    })
}

fn text_coverage_warnings_for_element(
    slide_id: &str,
    element_id: &str,
    kind: ElementKind,
    part: &str,
    core_kind: CoreElementKind,
    bounds: Option<&pptx_compose_core::pptx::shape::Bounds>,
) -> Vec<TextCoverageWarning> {
    if core_kind != CoreElementKind::Picture
        || !bounds.is_some_and(|bounds| bounds.cx > 0 && bounds.cy > 0)
    {
        return Vec::new();
    }

    vec![TextCoverageWarning {
        code: "possible_image_text_uneditable".to_owned(),
        slide_id: slide_id.to_owned(),
        element_id: element_id.to_owned(),
        kind,
        part: part.to_owned(),
        reason: "image_text_not_extractable".to_owned(),
        detail: "This picture may contain visible text baked into image pixels or vector artwork. V1 does not OCR or edit that text, so find-text only searches accessibility metadata and XML text, not the image content.".to_owned(),
    }]
}

fn chart_text_coverage_warning(
    slide_id: &str,
    element_id: &str,
    kind: ElementKind,
    part: &str,
    reason: &str,
) -> TextCoverageWarning {
    TextCoverageWarning {
        code: "chart_text_unsupported".to_owned(),
        slide_id: slide_id.to_owned(),
        element_id: element_id.to_owned(),
        kind,
        part: part.to_owned(),
        reason: reason.to_owned(),
        detail: "Some visible chart text is backed by chart XML string caches or workbook formulas. V1 exposes editable DrawingML rich text such as simple titles, but preserves cache/workbook-backed labels unless all companion state can be kept consistent.".to_owned(),
    }
}

fn diagram_text_coverage_warning_detail(reason: &str) -> &'static str {
    if reason == "diagram_drawing_cache_text_absent" {
        "V1 does not edit SmartArt text when the related drawing cache contains no DrawingML text paragraphs to keep synchronized with diagram data."
    } else {
        "V1 does not edit SmartArt text when rendered drawing-cache paragraphs differ from diagram data and no modelId mapping proves which data node owns each visible paragraph."
    }
}

fn project_placeholder(element: &XmlElement, layout_part: Option<&str>) -> Option<PlaceholderView> {
    let placeholder = own_placeholder(element)?;
    let placeholder_type = optional_attr(placeholder, "type").unwrap_or("body");
    Some(PlaceholderView {
        r#type: placeholder_type.to_owned(),
        idx: optional_attr(placeholder, "idx").and_then(parse_u32),
        source: "slide".to_owned(),
        layout_part: layout_part.map(str::to_owned),
    })
}

fn own_placeholder(element: &XmlElement) -> Option<&XmlElement> {
    let non_visual_properties = element
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .find(|child| {
            matches!(
                child.name.local_name.as_str(),
                "nvSpPr" | "nvPicPr" | "nvGraphicFramePr" | "nvGrpSpPr" | "nvCxnSpPr"
            )
        })?;
    child_element(non_visual_properties, "nvPr").and_then(|nv_pr| child_element(nv_pr, "ph"))
}

fn project_text_layout(element: &XmlElement) -> Option<TextLayoutView> {
    let tx_body = child_element(element, "txBody")?;
    let body_pr = child_element(tx_body, "bodyPr").map(project_body_pr);
    let paragraph_defaults = first_descendant(tx_body, "pPr").and_then(project_paragraph_defaults);
    Some(TextLayoutView {
        body_pr,
        paragraph_defaults,
        style_confidence: StyleConfidence::DirectOnly,
    })
}

fn project_body_pr(body_pr: &XmlElement) -> BodyPrView {
    BodyPrView {
        wrap: optional_attr(body_pr, "wrap").map(str::to_owned),
        anchor: optional_attr(body_pr, "anchor").map(str::to_owned),
        inset_l: optional_attr(body_pr, "lIns").and_then(parse_i64),
        inset_r: optional_attr(body_pr, "rIns").and_then(parse_i64),
        inset_t: optional_attr(body_pr, "tIns").and_then(parse_i64),
        inset_b: optional_attr(body_pr, "bIns").and_then(parse_i64),
        autofit: autofit_kind(body_pr),
    }
}

fn project_paragraph_defaults(p_pr: &XmlElement) -> Option<ParagraphDefaultsView> {
    let align = optional_attr(p_pr, "algn").map(str::to_owned);
    align.map(|align| ParagraphDefaultsView { align: Some(align) })
}

fn autofit_kind(body_pr: &XmlElement) -> AutofitKind {
    if child_element(body_pr, "noAutofit").is_some() {
        AutofitKind::NoAutofit
    } else if child_element(body_pr, "normAutofit")
        .or_else(|| child_element(body_pr, "normAutoFit"))
        .is_some()
    {
        AutofitKind::NormAutoFit
    } else if child_element(body_pr, "spAutoFit").is_some() {
        AutofitKind::ShapeAutoFit
    } else {
        AutofitKind::Unknown
    }
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

#[cfg(test)]
fn project_text(tx_body: &XmlElement) -> TextView {
    let body = read_text_body(tx_body);
    project_text_body(body)
}

fn project_text_body(body: pptx_compose_core::pptx::text::TextBody) -> TextView {
    let text_hash = text_hash::text_hash(&body.normalized);
    let full_plain = body.plain.clone();
    let full_normalized = body.normalized.clone();
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
            let full_paragraph_text = paragraph.text.clone();
            let paragraph_text = truncate_text(
                paragraph.text,
                PARAGRAPH_PREVIEW_CHARS,
                "Use element_detail for this element to inspect the full paragraph text.",
            );
            Paragraph {
                text: paragraph_text.0,
                full_text: full_paragraph_text,
                runs: paragraph
                    .runs
                    .into_iter()
                    .map(|run| {
                        let full_run_text = run.text.clone();
                        let run_text = truncate_text(
                            run.text,
                            RUN_PREVIEW_CHARS,
                            "Use element_detail for this element to inspect the full run text.",
                        );
                        Run {
                            text: run_text.0,
                            full_text: full_run_text,
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
        full_plain,
        full_normalized,
        paragraphs,
        text_hash,
        truncation,
    }
}

fn project_table(element: &XmlElement) -> (Option<TableView>, Option<TextView>) {
    let Some(table) = first_descendant(element, "tbl") else {
        return (None, None);
    };
    let mut rows = Vec::new();
    let mut bodies = Vec::new();
    for (row_index, row_element) in child_elements(table, "tr").enumerate() {
        let Ok(row_index) = u32::try_from(row_index) else {
            break;
        };
        let mut cells = Vec::new();
        for (col_index, cell_element) in child_elements(row_element, "tc").enumerate() {
            let Ok(col_index) = u32::try_from(col_index) else {
                break;
            };
            let body = child_element(cell_element, "txBody").map(read_text_body);
            if let Some(body) = &body
                && !body.normalized.is_empty()
            {
                bodies.push(body.clone());
            }
            let editable = table_cell_editable(cell_element, body.as_ref());
            cells.push(TableCell {
                row: row_index,
                col: col_index,
                editable,
                text: body.map(project_text_body),
            });
        }
        rows.push(TableRow {
            row: row_index,
            cells,
        });
    }
    let table = TableView { rows };
    let text = if bodies.is_empty() {
        None
    } else {
        Some(project_text_body(merge_text_bodies(bodies)))
    };
    (Some(table), text)
}

fn table_cell_editable(
    cell_element: &XmlElement,
    body: Option<&pptx_compose_core::pptx::text::TextBody>,
) -> Editable {
    let text = body.and_then(|body| {
        (!body.normalized.is_empty()).then(|| {
            if is_merged_or_spanned_cell(cell_element) {
                EditableSupport {
                    supported: false,
                    reason: Some("merged_or_spanned_cell".to_owned()),
                }
            } else {
                EditableSupport {
                    supported: true,
                    reason: None,
                }
            }
        })
    });
    Editable {
        text,
        bounds: None,
        alt_text: None,
        image: None,
    }
}

fn is_merged_or_spanned_cell(cell_element: &XmlElement) -> bool {
    attr_u32_gt_one(cell_element, "gridSpan")
        || attr_u32_gt_one(cell_element, "rowSpan")
        || attr_present(cell_element, "vMerge")
        || child_element(cell_element, "vMerge").is_some()
}

fn attr_u32_gt_one(element: &XmlElement, local_name: &str) -> bool {
    optional_attr(element, local_name)
        .and_then(parse_u32)
        .is_some_and(|value| value > 1)
}

fn attr_present(element: &XmlElement, local_name: &str) -> bool {
    optional_attr(element, local_name).is_some()
}

struct TextProjection {
    text: Option<TextView>,
    support: Option<EditableSupport>,
    uneditable_related_text_reason: Option<String>,
}

fn projected_element_text(
    element: &XmlElement,
    core_kind: CoreElementKind,
    slide_rels: &RelationshipSet,
    package: &Package,
) -> Result<TextProjection, JsonError> {
    let projection =
        project_element_text(element, core_kind, slide_rels, package).map_err(core_error)?;
    let support = if projection.diagram_cache_mapping_unsupported {
        Some(EditableSupport {
            supported: false,
            reason: Some("diagram_drawing_cache_mapping_unsupported".to_owned()),
        })
    } else if projection.diagram_drawing_cache_text_absent {
        Some(EditableSupport {
            supported: false,
            reason: Some("diagram_drawing_cache_text_absent".to_owned()),
        })
    } else if projection.text.is_none() && projection.has_uneditable_chart_cache_text {
        Some(EditableSupport {
            supported: false,
            reason: Some("chart_cache_workbook_sync_unsupported".to_owned()),
        })
    } else {
        None
    };
    let uneditable_related_text_reason = if projection.diagram_cache_mapping_unsupported {
        None
    } else {
        projection
            .has_uneditable_chart_cache_text
            .then(|| "chart_cache_workbook_sync_unsupported".to_owned())
    };
    Ok(TextProjection {
        text: projection.text.map(project_text_body),
        support,
        uneditable_related_text_reason,
    })
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
            pptx_compose_core::validation::ValidationStatus::ValidWithErrors => {
                ValidationStatus::ValidWithErrors
            }
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
    UnresolvedPicture,
    UnresolvedRelationship,
}

fn unresolved_picture_support(
    element: &XmlElement,
    slide_rels: &RelationshipSet,
) -> ImageEditSupport {
    let Some(blip) = first_descendant(element, "blip") else {
        return ImageEditSupport::UnresolvedPicture;
    };
    let Some(embed_rel_id) = optional_attr(blip, "embed") else {
        return ImageEditSupport::UnresolvedPicture;
    };
    if slide_rels.get(embed_rel_id).is_none() {
        return ImageEditSupport::UnresolvedRelationship;
    }
    ImageEditSupport::UnresolvedPicture
}

fn editable(
    core_kind: CoreElementKind,
    has_text: bool,
    has_cnvpr: bool,
    image_support: ImageEditSupport,
    text_support: Option<EditableSupport>,
) -> Editable {
    let (image_supported, image_reason) = match image_support {
        ImageEditSupport::Embedded => (true, None),
        ImageEditSupport::ExternalLink => (false, Some("external_link".to_owned())),
        ImageEditSupport::UnresolvedPicture => (false, Some("unresolved_picture".to_owned())),
        ImageEditSupport::UnresolvedRelationship => {
            (false, Some("unresolved_relationship".to_owned()))
        }
    };
    let (bounds_supported, bounds_reason) = bounds_edit_support(core_kind);
    let text = if text_support.is_some() {
        text_support
    } else if has_text && core_kind.supports_replace_text() {
        Some(EditableSupport {
            supported: true,
            reason: None,
        })
    } else if has_text && core_kind == CoreElementKind::GraphicFrameTable {
        Some(EditableSupport {
            supported: true,
            reason: Some("table_cell_coordinates_required".to_owned()),
        })
    } else if has_text {
        Some(EditableSupport {
            supported: false,
            reason: Some("unsupported_kind".to_owned()),
        })
    } else {
        None
    };
    let bounds = (bounds_supported && has_cnvpr).then_some(EditableSupport {
        supported: bounds_supported,
        reason: bounds_reason.map(str::to_owned),
    });
    let alt_text = has_cnvpr.then_some(EditableSupport {
        supported: true,
        reason: None,
    });
    let image = (core_kind == CoreElementKind::Picture).then_some(EditableSupport {
        supported: image_supported,
        reason: image_reason,
    });

    Editable {
        text,
        bounds,
        alt_text,
        image,
    }
}

fn bounds_edit_support(kind: CoreElementKind) -> (bool, Option<&'static str>) {
    if kind.supports_move_resize() {
        return (true, None);
    }

    let reason = match kind {
        CoreElementKind::Group => "group",
        CoreElementKind::Connector => "connector",
        CoreElementKind::Other => "unbounded",
        _ => "unbounded",
    };
    (false, Some(reason))
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

fn notes_body_shape_tx_body(element: &XmlElement) -> Option<(&XmlElement, &XmlElement)> {
    for child in element.children.iter().filter_map(XmlNode::as_element) {
        if child.name.local_name == "sp" {
            let has_body_placeholder = first_descendant(child, "ph").is_some_and(|ph| {
                ph.attributes.iter().any(|attribute| {
                    attribute.name.local_name == "type" && attribute.value == "body"
                })
            });
            if has_body_placeholder && let Some(tx_body) = child_element(child, "txBody") {
                return Some((child, tx_body));
            }
        }
        if let Some(descendant) = notes_body_shape_tx_body(child) {
            return Some(descendant);
        }
    }
    None
}

fn child_elements<'a>(
    element: &'a XmlElement,
    local_name: &'a str,
) -> impl Iterator<Item = &'a XmlElement> {
    element
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .filter(move |child| child.name.local_name == local_name)
}

fn child_element<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a XmlElement> {
    element
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .find(|child| child.name.local_name == local_name)
}

fn optional_attr<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local_name == local_name)
        .map(|attribute| attribute.value.as_str())
}

fn parse_i64(value: &str) -> Option<i64> {
    value.parse().ok()
}

fn parse_u32(value: &str) -> Option<u32> {
    value.parse().ok()
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
fn media_metadata_cursor_walk_matches_full_inventory_without_duplicates() {
    let pkg = package_from_pptx_bytes(include_bytes!(
        "../../../../fixtures/real-world/worldbank-macro-economic-update.pptx"
    ))
    .expect("fixture package parses");
    let full = build_view(
        &pkg,
        ViewRequest {
            mode: ViewMode::MediaMetadata,
            include_elements: false,
            slide_id: None,
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: Some(100),
        },
    )
    .expect("full media metadata builds");
    let expected = full["media"]
        .as_array()
        .expect("full media array")
        .iter()
        .map(|media| {
            media["media_part"]
                .as_str()
                .expect("media_part is a string")
                .to_owned()
        })
        .collect::<Vec<_>>();

    let mut actual = Vec::new();
    let mut cursor = None;
    loop {
        let page = build_view(
            &pkg,
            ViewRequest {
                mode: ViewMode::MediaMetadata,
                include_elements: false,
                slide_id: None,
                slide_ids: Vec::new(),
                element_id: None,
                cursor,
                limit: Some(2),
            },
        )
        .expect("paged media metadata builds");
        actual.extend(
            page["media"]
                .as_array()
                .expect("paged media array")
                .iter()
                .map(|media| {
                    media["media_part"]
                        .as_str()
                        .expect("media_part is a string")
                        .to_owned()
                }),
        );
        cursor = page["view"]["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            assert_eq!(page["view"]["truncated"], false);
            break;
        }
    }

    let unique = actual.iter().collect::<BTreeSet<_>>();
    assert_eq!(actual.len(), unique.len());
    assert_eq!(actual, expected);
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

    assert_eq!(elements.len(), 3);
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
    assert_eq!(embedded["editable"].get("text"), None);
    assert_eq!(embedded["image"]["relationship_id"], "rEmbed");

    let linked = elements
        .iter()
        .find(|element| element["xml_location"]["cnvpr_name"] == "Linked Picture")
        .expect("linked picture is projected");
    assert_eq!(linked["editable"]["image"]["supported"], false);
    assert_eq!(linked["editable"]["image"]["reason"], "external_link");
    assert_eq!(linked["editable"].get("text"), None);
    assert_eq!(linked.get("image"), None);

    let dual = elements
        .iter()
        .find(|element| element["xml_location"]["cnvpr_name"] == "Dual Embed Link Picture")
        .expect("dual embed/link picture is projected");
    assert_eq!(dual["editable"]["image"]["supported"], false);
    assert_eq!(dual["editable"]["image"]["reason"], "external_link");
    assert_eq!(dual["editable"].get("text"), None);
    assert_eq!(dual.get("image"), None);

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
fn unresolved_picture_projection_reports_stable_image_reason() {
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
    const SLIDE_XML: &[u8] = br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:pic><p:nvPicPr><p:cNvPr id="5" name="Missing Relationship Picture"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rMissing"/></p:blipFill><p:spPr/></p:pic></p:spTree></p:cSld></p:sld>"#;

    let presentation_part = PartName::from_zip_entry("ppt/presentation.xml").expect("part name");

    let mut package = Package::new();
    package
        .insert_zip_entry("[Content_Types].xml", content_types_xml().to_vec())
        .expect("content types part inserts");
    package
        .insert_zip_entry("ppt/presentation.xml", presentation_xml().to_vec())
        .expect("presentation part inserts");
    package
        .insert_zip_entry("ppt/slides/slide1.xml", SLIDE_XML.to_vec())
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
    let picture = value["slides"][0]["elements"]
        .as_array()
        .expect("elements array")
        .iter()
        .find(|element| element["xml_location"]["cnvpr_name"] == "Missing Relationship Picture")
        .expect("picture is projected despite unresolved media");

    assert_eq!(picture["editable"]["image"]["supported"], false);
    assert_eq!(
        picture["editable"]["image"]["reason"],
        "unresolved_relationship"
    );
    assert_eq!(picture.get("image"), None);
}

#[cfg(test)]
#[test]
fn editable_maps_are_kind_appropriate_for_text_and_picture_elements() {
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

    let mut package = Package::new();
    package
        .insert_zip_entry("[Content_Types].xml", content_types_xml().to_vec())
        .expect("content types part inserts");
    package
        .insert_zip_entry("ppt/presentation.xml", presentation_xml().to_vec())
        .expect("presentation part inserts");
    package
        .insert_zip_entry("ppt/slides/slide1.xml", editable_map_slide_xml().to_vec())
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
        RelationshipSource::Part(slide_part),
        "rEmbed",
        IMAGE_REL_TYPE,
        "../media/image1.png",
    ));

    let pkg = PresentationDocument::open(package).expect("presentation opens");
    let value = build_view(&pkg, request_for(&pkg, ViewMode::SlideDetail, None))
        .expect("slide detail builds");
    let elements = value["slides"][0]["elements"]
        .as_array()
        .expect("elements array");

    let text = elements
        .iter()
        .find(|element| element["xml_location"]["cnvpr_name"] == "Editable Text")
        .expect("text box is projected");
    assert_eq!(
        text["editable"],
        serde_json::json!({
            "text": { "supported": true },
            "bounds": { "supported": true },
            "alt_text": { "supported": true }
        })
    );

    let image = elements
        .iter()
        .find(|element| element["xml_location"]["cnvpr_name"] == "Editable Picture")
        .expect("picture is projected");
    assert_eq!(
        image["editable"],
        serde_json::json!({
            "bounds": { "supported": true },
            "alt_text": { "supported": true },
            "image": { "supported": true }
        })
    );
    assert_eq!(
        image["text_coverage_warnings"][0]["code"],
        "possible_image_text_uneditable"
    );
    assert_eq!(
        image["text_coverage_warnings"][0]["reason"],
        "image_text_not_extractable"
    );

    let matches = find_text(
        &pkg,
        FindTextRequest {
            query: "Quarterly".to_owned(),
            scope: FindTextScope::Slide {
                slide_id: "slide-1".to_owned(),
            },
            cursor: None,
            limit: None,
        },
    )
    .expect("find-text searches scoped slide");
    assert_eq!(matches.matches.len(), 1);
    assert_eq!(matches.warnings.len(), 1);
    assert_eq!(
        matches.warnings[0].element_id,
        image["id"].as_str().expect("image id is a string")
    );
    assert_eq!(matches.warnings[0].code, "possible_image_text_uneditable");
}

#[cfg(test)]
#[test]
fn speaker_notes_are_projected_and_find_text_returns_slide_selector() {
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
    let slide_part = PartName::from_zip_entry("ppt/slides/slide1.xml").expect("part name");

    let mut package = Package::new();
    package
        .insert_zip_entry("[Content_Types].xml", content_types_xml().to_vec())
        .expect("content types part inserts");
    package
        .insert_zip_entry("ppt/presentation.xml", presentation_xml().to_vec())
        .expect("presentation part inserts");
    package
        .insert_zip_entry("ppt/slides/slide1.xml", empty_slide_xml().to_vec())
        .expect("slide part inserts");
    package
        .insert_zip_entry(
            "ppt/notesSlides/notesSlide1.xml",
            notes_slide_xml().to_vec(),
        )
        .expect("notes part inserts");
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
        RelationshipSource::Part(slide_part),
        "rNotes",
        NOTES_SLIDE_REL_TYPE,
        "../notesSlides/notesSlide1.xml",
    ));

    let pkg = PresentationDocument::open(package).expect("presentation opens");
    let value = build_view(&pkg, request_for(&pkg, ViewMode::SlideDetail, None))
        .expect("slide detail builds");
    let notes = value["slides"][0]["elements"]
        .as_array()
        .expect("elements array")
        .iter()
        .find(|element| element["id"] == "slide-1:notes")
        .expect("speaker notes are projected");
    assert_eq!(notes["part"], "ppt/notesSlides/notesSlide1.xml");
    assert_eq!(notes["editable"]["text"]["supported"], true);
    assert_eq!(notes["text"]["plain"], "Presenter cue");

    let matches = find_text(
        &pkg,
        FindTextRequest {
            query: "Presenter cue".to_owned(),
            scope: FindTextScope::Deck,
            cursor: None,
            limit: None,
        },
    )
    .expect("find-text searches speaker notes");
    assert_eq!(matches.matches.len(), 1);
    let note_match = &matches.matches[0];
    assert_eq!(note_match.element_id, "slide-1:notes");
    assert_eq!(note_match.part, "ppt/notesSlides/notesSlide1.xml");
    assert_eq!(note_match.selector.selector_type, "slide_id");
    assert_eq!(note_match.selector.id, "slide-1");
    assert!(note_match.selector.run.is_some());
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
fn table_cells_are_exposed_as_addressable_text() {
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
        .insert_zip_entry("ppt/slides/slide1.xml", table_slide_xml().to_vec())
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
    let table = value["slides"][0]["elements"]
        .as_array()
        .expect("elements array")
        .iter()
        .find(|element| element["kind"] == "table")
        .expect("table is projected");

    assert_eq!(table["editable"]["text"]["supported"], true);
    assert_eq!(
        table["editable"]["text"]["reason"],
        "table_cell_coordinates_required"
    );
    assert_eq!(
        table["text"]["plain"],
        "Header A\nHeader B\nFocus Area\nStrategic Objective\nMerged Label"
    );
    assert_eq!(table["table"]["rows"][0]["cells"][0]["row"], 0);
    assert_eq!(table["table"]["rows"][0]["cells"][0]["col"], 0);
    assert_eq!(
        table["table"]["rows"][0]["cells"][0]["editable"]["text"]["supported"],
        true
    );
    assert_eq!(
        table["table"]["rows"][1]["cells"][1]["text"]["paragraphs"][0]["runs"][0]["text"],
        "Strategic Objective"
    );
    assert_eq!(
        table["table"]["rows"][2]["cells"][0]["editable"]["text"]["supported"],
        false
    );
    assert_eq!(
        table["table"]["rows"][2]["cells"][0]["editable"]["text"]["reason"],
        "merged_or_spanned_cell"
    );

    let matches = find_text(
        &pkg,
        FindTextRequest {
            query: "Strategic".to_owned(),
            scope: FindTextScope::Deck,
            cursor: None,
            limit: None,
        },
    )
    .expect("find_text searches table cells");
    assert_eq!(matches.matches.len(), 1);
    assert_eq!(matches.matches[0].element_id, table["id"].as_str().unwrap());
    assert_eq!(
        matches.matches[0].cell,
        Some(TableCellRef { row: 1, col: 1 })
    );
    assert_eq!(matches.matches[0].matched_text, "Strategic");

    let merged_matches = find_text(
        &pkg,
        FindTextRequest {
            query: "Merged Label".to_owned(),
            scope: FindTextScope::Deck,
            cursor: None,
            limit: None,
        },
    )
    .expect("find_text searches patch-ready table cells");
    assert!(
        merged_matches.matches.is_empty(),
        "merged or spanned cells must not produce patch-ready selectors"
    );
}

#[cfg(test)]
#[test]
fn bounds_editability_matches_move_resize_supported_kinds() {
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
        .insert_zip_entry(
            "ppt/slides/slide1.xml",
            group_connector_slide_xml().to_vec(),
        )
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

    for (name, bounds) in [
        ("Unsupported Group", None),
        ("Unsupported Connector", None),
        (
            "Nested Shape",
            Some(serde_json::json!({ "supported": true })),
        ),
    ] {
        let element = elements
            .iter()
            .find(|element| element["xml_location"]["cnvpr_name"] == name)
            .unwrap_or_else(|| panic!("{name} should be projected"));
        match bounds {
            Some(bounds) => assert_eq!(element["editable"]["bounds"], bounds),
            None => assert_eq!(element["editable"].get("bounds"), None),
        }
    }

    assert_eq!(
        bounds_edit_support(CoreElementKind::Other),
        (false, Some("unbounded"))
    );
}

#[cfg(test)]
#[test]
fn alt_text_editability_matches_cnvpr_presence() {
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
        .insert_zip_entry("ppt/slides/slide1.xml", no_cnvpr_slide_xml().to_vec())
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

    let supported = elements
        .iter()
        .find(|element| element["xml_location"]["cnvpr_name"] == "Supported Shape")
        .expect("shape with cNvPr is projected");
    assert_eq!(supported["editable"]["alt_text"]["supported"], true);
    assert_eq!(supported["editable"]["alt_text"].get("reason"), None);

    let unsupported = elements
        .iter()
        .find(|element| element["xml_location"]["cnvpr_id"] == 0)
        .expect("shape without cNvPr is projected");
    assert_eq!(unsupported["editable"].get("alt_text"), None);
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
fn empty_slide_xml() -> &'static [u8] {
    br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld></p:sld>"#
}

#[cfg(test)]
fn notes_slide_xml() -> &'static [u8] {
    br#"<p:notes xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="Slide Image Placeholder"/><p:cNvSpPr/><p:nvPr><p:ph type="sldImg"/></p:nvPr></p:nvSpPr><p:spPr/></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Notes Placeholder"/><p:cNvSpPr txBox="1"/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Presenter cue</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"#
}

#[cfg(test)]
fn picture_slide_xml() -> &'static [u8] {
    br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:pic><p:nvPicPr><p:cNvPr id="5" name="Embedded Picture"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rEmbed"/></p:blipFill><p:spPr/></p:pic><p:pic><p:nvPicPr><p:cNvPr id="6" name="Linked Picture"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:link="rLink"/></p:blipFill><p:spPr/></p:pic><p:pic><p:nvPicPr><p:cNvPr id="7" name="Dual Embed Link Picture"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rEmbed" r:link="rLink"/></p:blipFill><p:spPr/></p:pic></p:spTree></p:cSld></p:sld>"#
}

#[cfg(test)]
fn editable_map_slide_xml() -> &'static [u8] {
    br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="4" name="Editable Text"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1000" cy="1000"/></a:xfrm></p:spPr><p:txBody><a:p><a:r><a:t>Quarterly Results</a:t></a:r></a:p></p:txBody></p:sp><p:pic><p:nvPicPr><p:cNvPr id="5" name="Editable Picture"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rEmbed"/></p:blipFill><p:spPr><a:xfrm><a:off x="1000" y="0"/><a:ext cx="1000" cy="1000"/></a:xfrm></p:spPr></p:pic></p:spTree></p:cSld></p:sld>"#
}

#[cfg(test)]
fn graphic_frame_slide_xml() -> &'static [u8] {
    br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="7" name="Chart Frame"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"/></a:graphic></p:graphicFrame><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="8" name="Table Frame"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl/></a:graphicData></a:graphic></p:graphicFrame><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="9" name="Diagram Frame"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"/></a:graphic></p:graphicFrame><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="10" name="OLE Frame"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/presentationml/2006/ole"/></a:graphic></p:graphicFrame><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="11" name="Unknown Frame"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="http://example.invalid/customGraphic"/></a:graphic></p:graphicFrame></p:spTree></p:cSld></p:sld>"#
}

#[cfg(test)]
fn table_slide_xml() -> &'static [u8] {
    br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="8" name="Results Table"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tr><a:tc><a:txBody><a:p><a:r><a:t>Header A</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>Header B</a:t></a:r></a:p></a:txBody></a:tc></a:tr><a:tr><a:tc><a:txBody><a:p><a:r><a:t>Focus Area</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>Strategic Objective</a:t></a:r></a:p></a:txBody></a:tc></a:tr><a:tr><a:tc gridSpan="2"><a:txBody><a:p><a:r><a:t>Merged Label</a:t></a:r></a:p></a:txBody></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame></p:spTree></p:cSld></p:sld>"#
}

#[cfg(test)]
fn group_connector_slide_xml() -> &'static [u8] {
    br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:grpSp><p:nvGrpSpPr><p:cNvPr id="12" name="Unsupported Group"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1000" cy="1000"/></a:xfrm></p:grpSpPr><p:sp><p:nvSpPr><p:cNvPr id="13" name="Nested Shape"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1000" cy="1000"/></a:xfrm></p:spPr></p:sp></p:grpSp><p:cxnSp><p:nvCxnSpPr><p:cNvPr id="14" name="Unsupported Connector"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1000" cy="1000"/></a:xfrm></p:spPr></p:cxnSp></p:spTree></p:cSld></p:sld>"#
}

#[cfg(test)]
fn no_cnvpr_slide_xml() -> &'static [u8] {
    br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="15" name="Supported Shape"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1000" cy="1000"/></a:xfrm></p:spPr></p:sp><p:sp><p:spPr><a:xfrm><a:off x="1000" y="0"/><a:ext cx="1000" cy="1000"/></a:xfrm></p:spPr></p:sp></p:spTree></p:cSld></p:sld>"#
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
#[test]
fn paragraph_text_preserves_intra_paragraph_soft_breaks() {
    let xml = br#"<a:txBody xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
        <a:p>
            <a:r><a:t>Public Debt</a:t></a:r>
            <a:br/>
            <a:r><a:t>Association between High Public Debt and Low Growth</a:t></a:r>
        </a:p>
    </a:txBody>"#;
    let document = parse_document(xml).expect("text body XML parses");
    let text = project_text(document.root_element().expect("txBody root"));

    assert_eq!(
        text.plain,
        "Public Debt\nAssociation between High Public Debt and Low Growth"
    );
    assert_eq!(
        text.paragraphs[0].text,
        "Public Debt\nAssociation between High Public Debt and Low Growth"
    );
    assert_eq!(text.paragraphs[0].runs[0].text, "Public Debt");
    assert_eq!(
        text.paragraphs[0].runs[1].text,
        "Association between High Public Debt and Low Growth"
    );
}

#[cfg(test)]
#[test]
fn run_selector_for_span_accounts_for_inter_paragraph_separators() {
    let text = test_text_view(vec![
        vec!["Intro"],
        vec!["Second paragraph", " suffix"],
        vec!["Tail"],
    ]);
    assert_eq!(text.plain, "Intro\nSecond paragraph suffix\nTail");

    let selector = run_selector_for_span(&text, TextSpan { start: 6, end: 22 })
        .expect("second paragraph run resolves");

    assert_eq!(selector.paragraph_index, 1);
    assert_eq!(selector.run_index, 0);
    assert_eq!(selector.run_end_index, None);
    assert_eq!(
        selector.text_hash,
        Some(text_hash::text_hash("Second paragraph"))
    );
}

#[cfg(test)]
#[test]
fn run_selector_for_span_does_not_shift_to_later_run_after_newline() {
    let text = test_text_view(vec![vec!["X"], vec!["A", "B"]]);
    assert_eq!(text.plain, "X\nAB");

    let selector = run_selector_for_span(&text, TextSpan { start: 2, end: 3 })
        .expect("first run after separator resolves");

    assert_eq!(selector.paragraph_index, 1);
    assert_eq!(selector.run_index, 0);
    assert_eq!(selector.run_end_index, None);
    assert_eq!(selector.text_hash, Some(text_hash::text_hash("A")));
}

#[cfg(test)]
#[test]
fn find_text_searches_full_text_beyond_preview_truncation() {
    let prefix = "x".repeat(TEXT_PREVIEW_CHARS + 25);
    let query = "needle";
    let suffix = "y".repeat(32);
    let xml = format!(
        "<txBody><p><r><t>{prefix}</t></r><r><t>{query}</t></r><r><t>{suffix}</t></r></p></txBody>"
    );
    let document = parse_document(xml.as_bytes()).expect("text body XML parses");
    let text = project_text(document.root_element().expect("txBody root"));

    assert!(text.truncation.is_some());
    assert!(!text.plain.contains(query));

    let spans = find_query_spans(search_plain_text(&text), query).expect("query search succeeds");
    assert_eq!(
        spans,
        vec![TextSpan {
            start: 4121,
            end: 4127
        }]
    );
    assert_eq!(
        substring_by_char_span(search_plain_text(&text), spans[0]),
        query
    );

    let selector = run_selector_for_span(&text, spans[0]).expect("full run selector resolves");
    assert_eq!(selector.paragraph_index, 0);
    assert_eq!(selector.run_index, 1);
    assert_eq!(selector.run_end_index, None);
    assert_eq!(selector.text_hash, Some(text_hash::text_hash(query)));
}

#[cfg(test)]
fn test_text_view(paragraph_runs: Vec<Vec<&str>>) -> TextView {
    let paragraphs = paragraph_runs
        .into_iter()
        .map(|runs| {
            let text = runs.concat();
            Paragraph {
                full_text: text.clone(),
                text,
                runs: runs
                    .into_iter()
                    .map(|text| Run {
                        text: text.to_owned(),
                        full_text: text.to_owned(),
                        style_summary: empty_style_summary(),
                        truncation: None,
                    })
                    .collect(),
                truncation: None,
            }
        })
        .collect::<Vec<_>>();
    let plain = paragraphs
        .iter()
        .map(|paragraph| paragraph.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    TextView {
        normalized: plain.clone(),
        full_plain: plain.clone(),
        full_normalized: plain.clone(),
        text_hash: text_hash::text_hash(&plain),
        plain,
        paragraphs,
        truncation: None,
    }
}

#[cfg(test)]
fn empty_style_summary() -> StyleSummary {
    StyleSummary {
        font_size_pt: None,
        bold: None,
        italic: None,
        underline: None,
        font_color_rgb: None,
        latin_typeface: None,
        language: None,
    }
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
            text: None,
            bounds: Some(EditableSupport {
                supported: true,
                reason: None,
            }),
            alt_text: Some(EditableSupport {
                supported: true,
                reason: None,
            }),
            image: None,
        },
        fingerprint: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .to_owned(),
        accessibility: None,
        placeholder: None,
        text_layout: None,
        text: None,
        table: None,
        image: None,
        text_coverage_warnings: Vec::new(),
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
