use std::collections::BTreeMap;

use pptx_compose_core::{
    error::Error,
    opc::{
        package::Package,
        part_name::PartName,
        relationships::{RelationshipSet, RelationshipSource},
    },
    pptx::{
        ids::{
            ElementKind as CoreElementKind, SpTreePath, agent_element_id, paragraph_agent_id,
            run_agent_id,
        },
        picture::read_picture,
        presentation::PresentationDocument,
        shape::{Shape, ShapeKind, read_shape},
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
use serde_json::{Value, json};

use super::{
    AgentView, Bounds, Capabilities, Editable, EditableSupport, ElementKind, ElementView,
    ImageView, IntrinsicSizePx, Paragraph, PresentationView, Run, SlideView, StyleSummary,
    TextView, XmlLocation,
    pagination::{CursorScope, ViewMeta, default_limit, paginate},
};
use crate::{
    schema_versions::{AGENT_VIEW_SCHEMA, AGENT_VIEW_VERSION},
    schemas::{
        FindingCategory, FindingCode, FindingView, JsonError, Severity, Summary, ValidationStatus,
    },
};

pub type PptxPackage = PresentationDocument;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slide_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

pub fn build_view(pkg: &PptxPackage, req: ViewRequest) -> Result<Value, JsonError> {
    let context = ViewContext::new(pkg)?;
    let mode = req.mode.as_token();
    let limit = req
        .limit
        .unwrap_or_else(|| default_limit(mode).unwrap_or(20));
    let scope = CursorScope {
        document_id: &context.document_id,
        revision: context.revision,
        mode,
    };

    match req.mode {
        ViewMode::DeckSummary => {
            let view = context.agent_view(view_meta(mode, limit), 0, Vec::new(), scope_value(&req));
            let mut value = to_value(view)?;
            value["warnings"] = json!([]);
            Ok(value)
        }
        ViewMode::SlidePage => {
            let (page, meta, omitted_count) =
                paginate(&context.slides, limit, req.cursor.as_deref(), scope)?;
            let slides = page
                .into_iter()
                .map(|slide| slide.summary.clone())
                .collect::<Vec<_>>();
            Ok(to_value(context.agent_view(
                meta,
                omitted_count,
                slides,
                scope_value(&req),
            ))?)
        }
        ViewMode::SlideDetail => {
            let slide_id = req.slide_id.as_deref().ok_or_else(|| {
                JsonError::Projection("slide_detail requires slide_id.".to_owned())
            })?;
            let slide = context.slide(slide_id).ok_or_else(|| JsonError::NotFound {
                kind: "slide",
                id: slide_id.to_owned(),
            })?;
            Ok(to_value(context.agent_view(
                view_meta(mode, limit),
                0,
                vec![slide.detail.clone()],
                scope_value(&req),
            ))?)
        }
        ViewMode::ElementDetail => {
            let element_id = req.element_id.as_deref().ok_or_else(|| {
                JsonError::Projection("element_detail requires element_id.".to_owned())
            })?;
            let slide = context
                .slides
                .iter()
                .find(|slide| {
                    slide
                        .detail
                        .elements
                        .iter()
                        .any(|element| element.id == element_id)
                })
                .ok_or_else(|| JsonError::NotFound {
                    kind: "element",
                    id: element_id.to_owned(),
                })?;
            let element = slide
                .detail
                .elements
                .iter()
                .find(|element| element.id == element_id)
                .expect("slide lookup proved element exists")
                .clone();
            let mut detail = slide.summary.clone();
            detail.elements = vec![element];
            Ok(to_value(context.agent_view(
                view_meta(mode, limit),
                0,
                vec![detail],
                scope_value(&req),
            ))?)
        }
        ViewMode::MediaMetadata => {
            let (page, meta, omitted_count) =
                paginate(&context.media, limit, req.cursor.as_deref(), scope)?;
            let mut value =
                to_value(context.agent_view(meta, omitted_count, Vec::new(), scope_value(&req)))?;
            value["media"] = Value::Array(page.into_iter().cloned().collect());
            Ok(value)
        }
        ViewMode::ValidationReport => {
            let (page, meta, omitted_count) = paginate(
                &context.validation.findings,
                limit,
                req.cursor.as_deref(),
                scope,
            )?;
            let mut validation = context.validation.clone();
            validation.findings = page.into_iter().cloned().collect();
            let mut value =
                to_value(context.agent_view(meta, omitted_count, Vec::new(), scope_value(&req)))?;
            value["validation"] = to_value(validation)?;
            Ok(value)
        }
    }
}

#[derive(Clone)]
struct ViewContext {
    document_id: String,
    revision: u32,
    presentation_part: String,
    slide_count: u32,
    capabilities: Capabilities,
    slides: Vec<SlideProjection>,
    media: Vec<Value>,
    validation: ValidationProjection,
}

#[derive(Clone)]
struct SlideProjection {
    summary: SlideView,
    detail: SlideView,
}

#[derive(Clone, Serialize)]
struct ValidationProjection {
    status: ValidationStatus,
    summary: Summary,
    findings: Vec<FindingView>,
}

impl ViewContext {
    fn new(pkg: &PptxPackage) -> Result<Self, JsonError> {
        let document_id = package_document_id(pkg.package())?;
        let revision = u32::try_from(revision::on_open().value())
            .map_err(|err| JsonError::Projection(err.to_string()))?;
        let mut media = BTreeMap::<String, Value>::new();
        let slides = project_slides(pkg, &mut media)?;
        let validation = project_validation(pkg.package());
        let slide_count = u32::try_from(pkg.slides().len())
            .map_err(|err| JsonError::Projection(err.to_string()))?;

        Ok(Self {
            document_id,
            revision,
            presentation_part: trim_part(pkg.presentation().part_name.as_str()),
            slide_count,
            capabilities: default_capabilities(),
            slides,
            media: media.into_values().collect(),
            validation,
        })
    }

    fn slide(&self, slide_id: &str) -> Option<&SlideProjection> {
        self.slides.iter().find(|slide| slide.detail.id == slide_id)
    }

    fn agent_view(
        &self,
        view: ViewMeta,
        omitted_count: u32,
        slides: Vec<SlideView>,
        scope: Cpj,
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
        }
    }
}

fn project_slides(
    pkg: &PptxPackage,
    media: &mut BTreeMap<String, Value>,
) -> Result<Vec<SlideProjection>, JsonError> {
    let mut slides = Vec::new();
    for slide in pkg.slides() {
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
        let part_checksum = part_checksum(part.bytes());
        let layout_part = slide
            .layout
            .as_ref()
            .map(|layout| trim_part(layout.part_name.as_str()))
            .unwrap_or_default();
        let base = SlideView {
            id: slide.agent_id(),
            index: slide.agent_index,
            ppt_slide_id: Some(slide.id.value()),
            part: trim_part(slide.part_name.as_str()),
            relationship_id: presentation_relationship_id(pkg, &slide.part_name),
            layout_part,
            part_checksum,
            elements: Vec::new(),
        };
        let mut detail = base.clone();
        detail.elements = elements;
        slides.push(SlideProjection {
            summary: base,
            detail,
        });
    }
    Ok(slides)
}

#[allow(clippy::too_many_arguments)]
fn collect_elements(
    parent: &XmlElement,
    slide_id: &str,
    slide_part: PartName,
    slide_rels: &RelationshipSet,
    package: &Package,
    output: &mut Vec<ElementView>,
    media: &mut BTreeMap<String, Value>,
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
    media: &mut BTreeMap<String, Value>,
) -> Result<(), JsonError> {
    for (zero_based_index, child) in parent
        .children
        .iter()
        .filter_map(XmlNode::as_element)
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

#[allow(clippy::too_many_arguments)]
fn project_element(
    element: &XmlElement,
    slide_id: &str,
    slide_part: PartName,
    path: SpTreePath,
    core_kind: CoreElementKind,
    slide_rels: &RelationshipSet,
    package: &Package,
    media: &mut BTreeMap<String, Value>,
) -> Result<ElementView, JsonError> {
    let shape = read_shape(element, path.clone());
    let text = first_descendant(element, "txBody")
        .map(|tx_body| project_text(tx_body, slide_id, core_kind, &path));
    let text_hash = text.as_ref().map(|text| text.text_hash.clone());
    let image = if core_kind == CoreElementKind::Picture {
        match read_picture(element, path.clone(), slide_rels, package) {
            Ok(picture) if !picture.external => {
                let image = ImageView {
                    relationship_id: picture.embed_rel_id,
                    media_part: trim_part(picture.media_part.as_str()),
                    content_type: picture.content_type,
                    byte_length: picture.byte_length,
                    checksum: package
                        .parts()
                        .get(&picture.media_part)
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
                    .or_insert_with(|| serde_json::to_value(&image).expect("ImageView serializes"));
                Some(image)
            }
            Ok(_) | Err(_) => None,
        }
    } else {
        None
    };

    Ok(ElementView {
        id: agent_element_id(slide_id, core_kind, &path),
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
        editable: editable(core_kind, text.is_some(), image.is_some()),
        fingerprint: fingerprint(&FingerprintInput {
            kind: core_kind,
            part: slide_part,
            sp_tree_path: path.sp_tree_path,
            group_path: path.group_path,
            cnvpr_id: shape.cnvpr_id,
            text_hash,
        }),
        text,
        image,
    })
}

fn project_text(
    tx_body: &XmlElement,
    slide_id: &str,
    kind: CoreElementKind,
    path: &SpTreePath,
) -> TextView {
    let body = read_text_body(tx_body);
    let element_id = agent_element_id(slide_id, kind, path);
    let paragraphs = body
        .paragraphs
        .into_iter()
        .map(|paragraph| {
            let paragraph_id = paragraph_agent_id(&element_id, paragraph.index as usize);
            Paragraph {
                id: paragraph_id.clone(),
                text: paragraph.runs.iter().map(|run| run.text.as_str()).collect(),
                runs: paragraph
                    .runs
                    .into_iter()
                    .map(|run| Run {
                        id: run_agent_id(&paragraph_id, run.index as usize),
                        text: run.text,
                        style_summary: StyleSummary {
                            font_size_pt: None,
                            bold: None,
                        },
                    })
                    .collect(),
            }
        })
        .collect();
    TextView {
        plain: body.plain,
        normalized: body.normalized.clone(),
        paragraphs,
        text_hash: text_hash::text_hash(&body.normalized),
    }
}

fn project_validation(package: &Package) -> ValidationProjection {
    let outcome = validate_package(package, ValidationMode::NoEdit);
    ValidationProjection {
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
                    pptx_compose_core::validation::FindingCode::UnresolvedRelationshipReference => {
                        FindingCode::UnresolvedRelationshipReference
                    }
                    pptx_compose_core::validation::FindingCode::DuplicateRelationshipId => {
                        FindingCode::DuplicateRelationshipId
                    }
                    pptx_compose_core::validation::FindingCode::ExternalRelationshipNotChecked => {
                        FindingCode::ExternalRelationshipNotChecked
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
        operations: vec![
            "replace_text".to_owned(),
            "add_text_box".to_owned(),
            "move_resize_element".to_owned(),
            "set_alt_text".to_owned(),
            "add_image".to_owned(),
            "replace_image".to_owned(),
        ],
        media_content_types: vec!["image/png".to_owned(), "image/jpeg".to_owned()],
        units: "emu".to_owned(),
    }
}

fn editable(kind: CoreElementKind, has_text: bool, has_image: bool) -> Editable {
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
            supported: has_image,
            reason: (!matches!(kind, CoreElementKind::Picture)).then(|| "not_picture".to_owned()),
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
        ShapeKind::GraphicFrame => CoreElementKind::GraphicFrame,
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
        CoreElementKind::GraphicFrame => ElementKind::Chart,
        CoreElementKind::Shape | CoreElementKind::Connector | CoreElementKind::Other => {
            ElementKind::Shape
        }
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

fn core_error(error: Error) -> JsonError {
    JsonError::Projection(error.message().to_owned())
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
    let pkg = package_from_pptx_bytes(include_bytes!("../../../../fixtures/minimal.pptx"))
        .expect("fixture package parses");
    let slide_count = pkg.slides().len();

    let deck_summary = build_view(
        &pkg,
        ViewRequest {
            mode: ViewMode::DeckSummary,
            slide_id: None,
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
            slide_id: None,
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
        slide_id: matches!(mode, ViewMode::SlideDetail)
            .then(|| slide_id.clone())
            .flatten(),
        element_id: matches!(mode, ViewMode::ElementDetail)
            .then(|| element_id.clone())
            .flatten(),
        cursor: None,
        limit: None,
    }
}
