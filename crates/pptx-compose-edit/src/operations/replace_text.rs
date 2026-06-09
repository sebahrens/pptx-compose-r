use pptx_compose_core::{
    error::{Error, ErrorCode, ErrorLocation, Result},
    opc::{
        package::Package,
        part_name::PartName,
        relationships::{RelationshipSet, TargetMode},
    },
    pptx::{
        ids::ElementKind,
        table_style::{TableProperties, TableStyleCatalog},
        text::read_text_body,
    },
    provenance::text_hash,
    xml::{
        chars::is_xml_char,
        document::{QualifiedName, XmlAttribute, XmlDocument, XmlElement, XmlNode},
        parser::parse_document,
        writer::{WriteMode, WriteOptions, write_document},
    },
};
use pptx_compose_json::schemas::OperationTarget;
use serde_json::json;

use crate::{
    operations::{
        ResolvedElement, ResolvedNotesSlide, ResolvedTableCell, add_text_box::validate_run_style,
        is_real_shape_tree_child,
    },
    patch::{
        FitPolicy, FitPolicyMode, FormatPolicy, PatchEffects, ReplaceTextMode,
        ReplaceTextOperation, TextAlign, TextRunStyle,
    },
    selectors::RunSelector,
};

const DIAGRAM_DRAWING_REL_TYPE: &str =
    "http://schemas.microsoft.com/office/2007/relationships/diagramDrawing";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplaceText {
    pub operation_id: String,
    pub element_id: String,
    pub text: String,
    pub current_text_match: Option<String>,
    pub mode: ReplaceTextMode,
    pub format_policy: FormatPolicy,
    pub allow_formatting_simplification: bool,
    pub run: Option<RunSelector>,
    pub run_style: Option<TextRunStyle>,
    pub fit_policy: Option<FitPolicy>,
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
            allow_formatting_simplification: operation.allow_formatting_simplification,
            run: operation.run_selector().cloned(),
            run_style: operation.run_style.clone(),
            fit_policy: operation.fit_policy,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplaceNotesText {
    pub operation_id: String,
    pub slide_id: String,
    pub text: String,
    pub current_text_match: Option<String>,
    pub run: Option<RunSelector>,
}

impl From<&ReplaceTextOperation> for ReplaceNotesText {
    fn from(operation: &ReplaceTextOperation) -> Self {
        Self {
            operation_id: operation.operation_id.clone(),
            slide_id: operation.target_slide_id().to_owned(),
            text: operation.text.clone(),
            current_text_match: operation.current_text_match.clone(),
            run: operation.run_selector().cloned(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplaceTableCellText {
    pub operation_id: String,
    pub element_id: String,
    pub row: u32,
    pub col: u32,
    pub text: String,
    pub current_text_match: Option<String>,
    pub run: Option<RunSelector>,
}

impl From<&ReplaceTextOperation> for ReplaceTableCellText {
    fn from(operation: &ReplaceTextOperation) -> Self {
        let cell = operation.cell.as_ref().copied().unwrap_or_default();
        Self {
            operation_id: operation.operation_id.clone(),
            element_id: operation.target_element_id().to_owned(),
            row: cell.row,
            col: cell.col,
            text: operation.text.clone(),
            current_text_match: operation.current_text_match.clone(),
            run: operation.run_selector().cloned(),
        }
    }
}

impl ReplaceText {
    pub fn validate(&self, package: &Package, target: &ResolvedElement) -> Result<()> {
        self.validate_target(target)?;
        if is_related_text_target(target.kind) {
            let (part_name, _) = related_text_part(package, target, self)?;
            let part = package.parts().get(&part_name).ok_or_else(|| {
                Error::new(
                    ErrorCode::SelectorNotFound,
                    format!("Related text part {part_name} was not found."),
                )
                .with_location(self.location(Some(target)))
            })?;
            let document = parse_document(part.bytes()).map_err(|source| {
                Error::with_source(
                    source.code(),
                    format!("Could not parse related text part {part_name}."),
                    source,
                )
                .with_location(self.location(Some(target)))
            })?;
            let root = document.root_element().ok_or_else(|| {
                Error::malformed_xml("Related text XML does not contain a root element.")
                    .with_location(self.location(Some(target)))
            })?;
            if self.mode != ReplaceTextMode::RunScoped {
                return Err(Error::new(
                    ErrorCode::UnsupportedEdit,
                    "Chart and diagram text replacement requires mode run_scoped.",
                )
                .with_location(self.location(Some(target))));
            }
            let tx_body = related_text_body(root, self, target)?;
            return validate_run_scoped_text_body(self, target, &tx_body);
        }
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
        self.validate_text_body(target, element, tx_body)
    }

    pub fn apply(&self, package: &mut Package, target: &ResolvedElement) -> Result<PatchEffects> {
        self.validate_target(target)?;
        if is_related_text_target(target.kind) {
            return self.apply_related_text(package, target);
        }

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

        let rewrite = rewrite_text_body(&mut document, self, target)?;
        *part.bytes_mut() = write_document(
            &document,
            &WriteOptions {
                mode: WriteMode::Preserve,
            },
        )?;
        package.mark_dirty(part_name.clone());

        let mut warnings = Vec::new();
        if self.mode == ReplaceTextMode::WholeElement {
            warnings.push(json!({ "newline_mapping": "paragraph" }));
        }
        if rewrite.formatting_simplified {
            warnings.push(json!({
                "code": "formatting_simplified",
                "message": "Existing rich text structure could not be preserved exactly."
            }));
        }
        if let Some(fit_warning) = rewrite.fit_warning {
            warnings.push(fit_warning);
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
        if !target.kind.supports_replace_text() {
            return Err(Error::new(
                ErrorCode::UnsupportedEdit,
                "Target element is not text-capable.",
            )
            .with_location(self.location(Some(target))));
        }
        Ok(())
    }

    fn validate_text_body(
        &self,
        target: &ResolvedElement,
        element: &XmlElement,
        tx_body: &XmlElement,
    ) -> Result<()> {
        match self.mode {
            ReplaceTextMode::WholeElement => {
                self.validate_whole_element_text_body(target, tx_body)?;
                self.validate_whole_element_match(target, tx_body)?;
            }
            ReplaceTextMode::RunScoped => validate_run_scoped_text_body(self, target, tx_body)?,
        }
        self.validate_fit_policy(target, element, tx_body)
    }

    fn validate_fit_policy(
        &self,
        target: &ResolvedElement,
        element: &XmlElement,
        tx_body: &XmlElement,
    ) -> Result<()> {
        let Some(policy) = self.fit_policy else {
            return Ok(());
        };
        let estimate = estimate_text_fit(element, tx_body, &self.text);
        match policy.mode {
            FitPolicyMode::Preserve => Ok(()),
            FitPolicyMode::FailIfOverflow if estimate.status == FitStatus::Overflow => {
                Err(Error::new(
                    ErrorCode::UnsupportedEdit,
                    "replace_text fit_policy fail_if_overflow rejected likely text overflow.",
                )
                .with_location(self.location(Some(target))))
            }
            FitPolicyMode::ShrinkText if estimate.status == FitStatus::Overflow => {
                Err(Error::new(
                    ErrorCode::UnsupportedEdit,
                    "replace_text fit_policy shrink_text requires run-size rewriting that is not supported for this target in V1.",
                )
                .with_location(self.location(Some(target))))
            }
            FitPolicyMode::FailIfOverflow | FitPolicyMode::ShrinkText => Ok(()),
        }
    }

    fn fit_warning(&self, element: &XmlElement, tx_body: &XmlElement) -> Option<serde_json::Value> {
        if self
            .fit_policy
            .is_some_and(|policy| policy.mode != FitPolicyMode::Preserve)
        {
            return None;
        }
        let estimate = estimate_text_fit(element, tx_body, &self.text);
        (estimate.status == FitStatus::Overflow).then(|| {
            json!({
                "code": "text_overflow_risk",
                "message": "Conservative text-fit estimate predicts likely overflow.",
                "fit": estimate.to_json()
            })
        })
    }

    fn validate_whole_element_text_body(
        &self,
        target: &ResolvedElement,
        tx_body: &XmlElement,
    ) -> Result<()> {
        validate_xml_text(&self.text, false)
            .map_err(|error| error.with_location(self.location(Some(target))))?;
        if self.run.is_some() {
            return Err(Error::new(
                ErrorCode::InvalidInput,
                "selector.run is only valid when replace_text mode is run_scoped.",
            )
            .with_location(self.location(Some(target))));
        }
        if self.run_style.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedEdit,
                "replace_text.run_style is only valid when mode is run_scoped.",
            )
            .with_location(self.location(Some(target))));
        }
        if !self.allow_formatting_simplification && should_warn_formatting_simplified(tx_body, self)
        {
            return Err(Error::new(
                ErrorCode::UnsupportedEdit,
                "Whole-element replace_text would simplify rich text; set allow_formatting_simplification to confirm.",
            )
            .with_location(self.location(Some(target))));
        }
        Ok(())
    }

    fn validate_whole_element_match(
        &self,
        target: &ResolvedElement,
        tx_body: &XmlElement,
    ) -> Result<()> {
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

    fn apply_related_text(
        &self,
        package: &mut Package,
        target: &ResolvedElement,
    ) -> Result<PatchEffects> {
        if self.mode != ReplaceTextMode::RunScoped {
            return Err(Error::new(
                ErrorCode::UnsupportedEdit,
                "Chart and diagram text replacement requires mode run_scoped.",
            )
            .with_location(self.location(Some(target))));
        }
        let (part_name, _) = related_text_part(package, target, self)?;
        let drawing_part_name = diagram_drawing_mirror_part(package, target, &part_name);
        let part = package.parts_mut().get_mut(&part_name).ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                format!("Related text part {part_name} was not found."),
            )
            .with_location(self.location(Some(target)))
        })?;
        let mut document = parse_document(part.bytes()).map_err(|source| {
            Error::with_source(
                source.code(),
                format!("Could not parse related text part {part_name}."),
                source,
            )
            .with_location(self.location(Some(target)))
        })?;
        let root = root_element_mut(&mut document).ok_or_else(|| {
            Error::malformed_xml("Related text XML does not contain a root element.")
                .with_location(self.location(Some(target)))
        })?;
        let tx_body = related_text_body(root, self, target)?;
        validate_run_scoped_text_body(self, target, &tx_body)?;
        replace_related_run_scoped_text(root, self, target)?;

        *part.bytes_mut() = write_document(
            &document,
            &WriteOptions {
                mode: WriteMode::Preserve,
            },
        )?;
        package.mark_dirty(part_name.clone());

        let mut changed_parts = vec![part_name.zip_entry_name().to_owned()];
        let mut warnings = Vec::new();
        if target.kind == ElementKind::GraphicFrameDiagram {
            match drawing_part_name {
                Some(drawing_part_name) => {
                    if replace_diagram_drawing_mirror(package, &drawing_part_name, self, target)? {
                        changed_parts.push(drawing_part_name.zip_entry_name().to_owned());
                    } else {
                        warnings.push(diagram_drawing_mirror_stale_warning(
                            drawing_part_name.zip_entry_name(),
                        ));
                    }
                }
                None => warnings.push(diagram_drawing_mirror_stale_warning(
                    "unresolved diagram drawing mirror",
                )),
            }
        }

        Ok(PatchEffects {
            changed_parts,
            target: Some(OperationTarget {
                slide_id: target.slide_id.clone(),
                element_id: target.element_id.clone(),
                part: part_name.zip_entry_name().to_owned(),
            }),
            created_element_ids: Vec::new(),
            warnings,
        })
    }
}

impl ReplaceNotesText {
    pub fn validate(&self, package: &Package, target: &ResolvedNotesSlide) -> Result<()> {
        self.validate_target(target)?;
        let part = package.parts().get(&target.notes_part).ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                format!("Speaker-notes part {} was not found.", target.notes_part),
            )
            .with_location(self.location(Some(target)))
        })?;
        let document = parse_document(part.bytes()).map_err(|source| {
            Error::with_source(
                source.code(),
                format!("Could not parse speaker-notes part {}.", target.notes_part),
                source,
            )
            .with_location(self.location(Some(target)))
        })?;
        let root = document.root_element().ok_or_else(|| {
            Error::malformed_xml("Speaker-notes XML does not contain a root element.")
                .with_location(self.location(Some(target)))
        })?;
        let tx_body = notes_body_tx_body(root).ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                "Speaker-notes part does not contain a notes body text shape.",
            )
            .with_location(self.location(Some(target)))
        })?;
        validate_run_scoped_text_body(self, &self.target_element(target), tx_body)
    }

    pub fn apply(
        &self,
        package: &mut Package,
        target: &ResolvedNotesSlide,
    ) -> Result<PatchEffects> {
        self.validate_target(target)?;
        let part_name = target.notes_part.clone();
        let part = package.parts_mut().get_mut(&part_name).ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                format!("Speaker-notes part {part_name} was not found."),
            )
            .with_location(self.location(Some(target)))
        })?;
        let mut document = parse_document(part.bytes()).map_err(|source| {
            Error::with_source(
                source.code(),
                format!("Could not parse speaker-notes part {part_name}."),
                source,
            )
            .with_location(self.location(Some(target)))
        })?;
        let root = root_element_mut(&mut document).ok_or_else(|| {
            Error::malformed_xml("Speaker-notes XML does not contain a root element.")
                .with_location(self.location(Some(target)))
        })?;
        let tx_body = notes_body_tx_body_mut(root).ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                "Speaker-notes part does not contain a notes body text shape.",
            )
            .with_location(self.location(Some(target)))
        })?;
        let element_target = self.target_element(target);
        validate_run_scoped_text_body(self, &element_target, tx_body)?;
        replace_run_scoped_text(tx_body, self, &element_target)?;

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

    fn validate_target(&self, target: &ResolvedNotesSlide) -> Result<()> {
        if target.slide_id != self.slide_id {
            return Err(Error::new(
                ErrorCode::SelectorGuardFailed,
                "Resolved slide does not match replace_text slide_id.",
            )
            .with_location(self.location(Some(target))));
        }
        Ok(())
    }

    fn target_element(&self, target: &ResolvedNotesSlide) -> ResolvedElement {
        ResolvedElement {
            slide_id: target.slide_id.clone(),
            element_id: target.element_id.clone(),
            kind: ElementKind::TextBox,
            part: target.notes_part.clone(),
            sp_tree_path: Vec::new(),
            group_path: Vec::new(),
            cnvpr_id: None,
            text_hash: None,
            fingerprint: String::new(),
        }
    }

    fn location(&self, target: Option<&ResolvedNotesSlide>) -> ErrorLocation {
        ErrorLocation {
            part: target.map(|target| target.notes_part.zip_entry_name().to_owned()),
            slide_id: Some(
                target
                    .map(|target| target.slide_id.clone())
                    .unwrap_or_else(|| self.slide_id.clone()),
            ),
            element_id: target.map(|target| target.element_id.clone()),
            operation_id: Some(self.operation_id.clone()),
            operation: Some("replace_text".to_owned()),
            ..ErrorLocation::default()
        }
    }
}

fn is_related_text_target(kind: ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::GraphicFrameChart | ElementKind::GraphicFrameDiagram
    )
}

fn related_text_part(
    package: &Package,
    target: &ResolvedElement,
    operation: &ReplaceText,
) -> Result<(PartName, u32)> {
    let slide_part = package.parts().get(&target.part).ok_or_else(|| {
        Error::new(
            ErrorCode::SelectorNotFound,
            format!("Target slide part {} was not found.", target.part),
        )
        .with_location(operation.location(Some(target)))
    })?;
    let slide_document = parse_document(slide_part.bytes()).map_err(|source| {
        Error::with_source(
            source.code(),
            format!("Could not parse target slide part {}.", target.part),
            source,
        )
        .with_location(operation.location(Some(target)))
    })?;
    let element = target_element(&slide_document, target)
        .ok_or_else(|| operation.not_found("Target element path no longer resolves."))?;
    let slide_rels = package
        .relationships()
        .set_for(&target.part)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                "Target slide has no relationships for chart or diagram text.",
            )
            .with_location(operation.location(Some(target)))
        })?;
    let mut rel_ids = Vec::new();
    collect_relationship_ids(element, &mut rel_ids);
    for rel_id in rel_ids {
        let Some(part_name) = internal_related_part(slide_rels, &rel_id) else {
            continue;
        };
        let Some(part) = package.parts().get(&part_name) else {
            continue;
        };
        let document = parse_document(part.bytes()).map_err(|source| {
            Error::with_source(
                source.code(),
                format!("Could not parse related text part {part_name}."),
                source,
            )
            .with_location(operation.location(Some(target)))
        })?;
        let Some(root) = document.root_element() else {
            continue;
        };
        let paragraph_count = count_related_paragraphs(root);
        if paragraph_count > 0 {
            return Ok((part_name, paragraph_count));
        }
    }
    Err(Error::new(
        ErrorCode::UnsupportedEdit,
        "Target chart or diagram does not expose related DrawingML text.",
    )
    .with_location(operation.location(Some(target))))
}

fn internal_related_part(slide_rels: &RelationshipSet, rel_id: &str) -> Option<PartName> {
    let relationship = slide_rels.get(rel_id)?;
    if relationship.target_mode != TargetMode::Internal {
        return None;
    }
    relationship.resolved_target.clone()
}

fn diagram_drawing_mirror_part(
    package: &Package,
    target: &ResolvedElement,
    data_part_name: &PartName,
) -> Option<PartName> {
    if target.kind != ElementKind::GraphicFrameDiagram {
        return None;
    }
    let slide_rels = package.relationships().set_for(&target.part)?;
    let data_stem = diagram_part_stem(data_part_name)?;
    let candidates = slide_rels
        .rels
        .iter()
        .filter_map(|relationship| {
            if relationship.target_mode != TargetMode::Internal
                || relationship.rel_type != DIAGRAM_DRAWING_REL_TYPE
            {
                return None;
            }
            relationship.resolved_target.clone()
        })
        .collect::<Vec<_>>();
    candidates
        .iter()
        .find(|part_name| diagram_part_stem(part_name).as_deref() == Some(data_stem.as_str()))
        .cloned()
}

fn diagram_part_stem(part_name: &PartName) -> Option<String> {
    let file_name = part_name.zip_entry_name().rsplit('/').next()?;
    let stem = file_name.strip_suffix(".xml")?;
    stem.strip_prefix("data")
        .or_else(|| stem.strip_prefix("drawing"))
        .map(str::to_owned)
}

fn replace_diagram_drawing_mirror(
    package: &mut Package,
    drawing_part_name: &PartName,
    operation: &ReplaceText,
    target: &ResolvedElement,
) -> Result<bool> {
    let Some(part) = package.parts_mut().get_mut(drawing_part_name) else {
        return Ok(false);
    };
    let mut document = parse_document(part.bytes()).map_err(|source| {
        Error::with_source(
            source.code(),
            format!("Could not parse diagram drawing part {drawing_part_name}."),
            source,
        )
        .with_location(operation.location(Some(target)))
    })?;
    let Some(root) = root_element_mut(&mut document) else {
        return Ok(false);
    };
    let tx_body = match related_text_body(root, operation, target) {
        Ok(tx_body) => tx_body,
        Err(error) if error.code() == ErrorCode::UnsupportedEdit => return Ok(false),
        Err(error) => return Err(error),
    };
    if validate_run_scoped_text_body(operation, target, &tx_body).is_err() {
        return Ok(false);
    }
    replace_related_run_scoped_text(root, operation, target)?;
    *part.bytes_mut() = write_document(
        &document,
        &WriteOptions {
            mode: WriteMode::Preserve,
        },
    )?;
    package.mark_dirty(drawing_part_name.clone());
    Ok(true)
}

fn diagram_drawing_mirror_stale_warning(part: &str) -> serde_json::Value {
    json!({
        "code": "diagram_drawing_mirror_stale",
        "part": part,
        "message": "SmartArt diagram data text was edited, but the cached drawing mirror could not be synchronized."
    })
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

fn count_related_paragraphs(element: &XmlElement) -> u32 {
    if element.name.local_name == "p" && related_paragraph_has_text(element) {
        return 1;
    }
    element
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .map(count_related_paragraphs)
        .sum()
}

fn related_paragraph_has_text(paragraph: &XmlElement) -> bool {
    let tx_body = XmlElement {
        name: paragraph.name.clone(),
        attributes: Vec::new(),
        namespaces: paragraph.namespaces.clone(),
        children: vec![XmlNode::Element(paragraph.clone())],
    };
    !read_text_body(&tx_body).normalized.is_empty()
}

fn related_text_body(
    root: &XmlElement,
    operation: &ReplaceText,
    target: &ResolvedElement,
) -> Result<XmlElement> {
    let mut children = Vec::new();
    collect_related_paragraph_nodes(root, &mut children);
    if children.is_empty() {
        return Err(Error::new(
            ErrorCode::UnsupportedEdit,
            "Related chart or diagram part does not contain DrawingML text paragraphs.",
        )
        .with_location(operation.location(Some(target))));
    }
    Ok(element("a:txBody", &[], children))
}

fn collect_related_paragraph_nodes(element: &XmlElement, output: &mut Vec<XmlNode>) {
    if element.name.local_name == "p" && related_paragraph_has_text(element) {
        output.push(XmlNode::Element(element.clone()));
        return;
    }
    for child in element.children.iter().filter_map(XmlNode::as_element) {
        collect_related_paragraph_nodes(child, output);
    }
}

fn replace_related_run_scoped_text(
    root: &mut XmlElement,
    operation: &ReplaceText,
    target: &ResolvedElement,
) -> Result<()> {
    let run = operation.run.as_ref().ok_or_else(|| {
        Error::new(
            ErrorCode::InvalidInput,
            "run_scoped replace_text requires selector.run.",
        )
        .with_location(operation.location(Some(target)))
    })?;
    let mut current_paragraph = 0_u32;
    let paragraph = related_paragraph_mut(root, run.paragraph_index, &mut current_paragraph)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                "selector.run resolved to a missing paragraph.",
            )
            .with_location(operation.location(Some(target)))
        })?;
    let mut tx_body = element("a:txBody", &[], vec![XmlNode::Element(paragraph.clone())]);
    let mut scoped_run = run.clone();
    scoped_run.paragraph_index = 0;
    let scoped_operation = ReplaceText {
        run: Some(scoped_run),
        ..operation.clone()
    };
    replace_run_scoped_text(&mut tx_body, &scoped_operation, target)?;
    let replacement = tx_body
        .children
        .into_iter()
        .find_map(|node| match node {
            XmlNode::Element(element) => Some(element),
            _ => None,
        })
        .ok_or_else(|| {
            Error::new(
                ErrorCode::InternalError,
                "Related paragraph was not rewritten.",
            )
        })?;
    *paragraph = replacement;
    Ok(())
}

fn related_paragraph_mut<'a>(
    element: &'a mut XmlElement,
    target_index: u32,
    current_index: &mut u32,
) -> Option<&'a mut XmlElement> {
    if element.name.local_name == "p" && related_paragraph_has_text(element) {
        if *current_index == target_index {
            return Some(element);
        }
        *current_index = current_index.saturating_add(1);
        return None;
    }
    for child in element.children.iter_mut().filter_map(node_element_mut) {
        if let Some(paragraph) = related_paragraph_mut(child, target_index, current_index) {
            return Some(paragraph);
        }
    }
    None
}

impl ReplaceTableCellText {
    pub fn validate(&self, package: &Package, target: &ResolvedTableCell) -> Result<()> {
        self.validate_target(target)?;
        let part_name = target.element.part.clone();
        let part = package.parts().get(&part_name).ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                format!("Target slide part {part_name} was not found."),
            )
            .with_location(self.location(Some(&target.element)))
        })?;
        let document = parse_document(part.bytes()).map_err(|source| {
            Error::with_source(
                source.code(),
                format!("Could not parse target slide part {part_name}."),
                source,
            )
            .with_location(self.location(Some(&target.element)))
        })?;
        let element = target_element(&document, &target.element)
            .ok_or_else(|| self.not_found("Target table path no longer resolves."))?;
        let table = first_descendant(element, "tbl").ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                "Target table frame does not contain a DrawingML table.",
            )
            .with_location(self.location(Some(&target.element)))
        })?;
        consult_table_style_read_model(package, table, target);
        let cell = table_cell(table, self.row, self.col, self, &target.element)?;
        reject_merged_cell(cell, self, &target.element)?;
        let tx_body = child_element(cell, "txBody").ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                "Target table cell does not contain a text body.",
            )
            .with_location(self.location(Some(&target.element)))
        })?;
        validate_run_scoped_text_body(self, &target.element, tx_body)
    }

    pub fn apply(&self, package: &mut Package, target: &ResolvedTableCell) -> Result<PatchEffects> {
        self.validate_target(target)?;

        let part_name = target.element.part.clone();
        let catalog = TableStyleCatalog::from_package(package);
        let part = package.parts_mut().get_mut(&part_name).ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                format!("Target slide part {part_name} was not found."),
            )
            .with_location(self.location(Some(&target.element)))
        })?;

        let mut document = parse_document(part.bytes()).map_err(|source| {
            Error::with_source(
                source.code(),
                format!("Could not parse target slide part {part_name}."),
                source,
            )
            .with_location(self.location(Some(&target.element)))
        })?;

        let root = root_element_mut(&mut document).ok_or_else(|| {
            Error::malformed_xml("Slide XML does not contain a root element.")
                .with_location(self.location(Some(&target.element)))
        })?;
        let sp_tree = first_descendant_mut(root, "spTree").ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                "Target slide does not contain a shape tree.",
            )
            .with_location(self.location(Some(&target.element)))
        })?;
        let element =
            element_at_path_mut(sp_tree, &target.element.sp_tree_path).ok_or_else(|| {
                Error::new(
                    ErrorCode::SelectorNotFound,
                    "Target table path no longer resolves in the slide shape tree.",
                )
                .with_location(self.location(Some(&target.element)))
            })?;
        let table = first_descendant_mut(element, "tbl").ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                "Target table frame does not contain a DrawingML table.",
            )
            .with_location(self.location(Some(&target.element)))
        })?;
        consult_table_style_catalog(catalog.as_ref(), table, target);
        let cell = table_cell_mut(table, self.row, self.col, self, &target.element)?;
        reject_merged_cell(cell, self, &target.element)?;
        let tx_body = child_element_mut(cell, "txBody").ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                "Target table cell does not contain a text body.",
            )
            .with_location(self.location(Some(&target.element)))
        })?;
        validate_run_scoped_text_body(self, &target.element, tx_body)?;
        replace_run_scoped_text(tx_body, self, &target.element)?;

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
                slide_id: target.element.slide_id.clone(),
                element_id: target.element.element_id.clone(),
                part: part_name.zip_entry_name().to_owned(),
            }),
            created_element_ids: Vec::new(),
            warnings: Vec::new(),
        })
    }

    fn validate_target(&self, target: &ResolvedTableCell) -> Result<()> {
        if target.element.element_id != self.element_id {
            return Err(Error::new(
                ErrorCode::SelectorGuardFailed,
                "Resolved element does not match replace_text element_id.",
            )
            .with_location(self.location(Some(&target.element))));
        }
        if target.element.kind != ElementKind::GraphicFrameTable {
            return Err(Error::new(
                ErrorCode::UnsupportedEdit,
                "Target element is not a table graphic frame.",
            )
            .with_location(self.location(Some(&target.element))));
        }
        if target.row != self.row || target.col != self.col {
            return Err(Error::new(
                ErrorCode::SelectorGuardFailed,
                "Resolved table cell does not match replace_text cell coordinates.",
            )
            .with_location(self.location(Some(&target.element))));
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

trait RunScopedTextOperation {
    fn text(&self) -> &str;
    fn current_text_match(&self) -> Option<&str>;
    fn run_selector(&self) -> Option<&RunSelector>;
    fn location(&self, target: Option<&ResolvedElement>) -> ErrorLocation;
    fn missing_run_message(&self) -> &'static str;
    fn newline_message(&self) -> &'static str;
    fn match_guard_message(&self) -> &'static str;
    fn allow_default_run(&self) -> bool;
    fn run_style(&self) -> Option<&TextRunStyle> {
        None
    }
}

impl RunScopedTextOperation for ReplaceText {
    fn text(&self) -> &str {
        &self.text
    }

    fn current_text_match(&self) -> Option<&str> {
        self.current_text_match.as_deref()
    }

    fn run_selector(&self) -> Option<&RunSelector> {
        self.run.as_ref()
    }

    fn location(&self, target: Option<&ResolvedElement>) -> ErrorLocation {
        ReplaceText::location(self, target)
    }

    fn missing_run_message(&self) -> &'static str {
        "run_scoped replace_text requires selector.run."
    }

    fn newline_message(&self) -> &'static str {
        "run_scoped replace_text text must not contain line-break characters."
    }

    fn match_guard_message(&self) -> &'static str {
        "replace_text match guard did not match current run text."
    }

    fn allow_default_run(&self) -> bool {
        false
    }

    fn run_style(&self) -> Option<&TextRunStyle> {
        self.run_style.as_ref()
    }
}

impl RunScopedTextOperation for ReplaceNotesText {
    fn text(&self) -> &str {
        &self.text
    }

    fn current_text_match(&self) -> Option<&str> {
        self.current_text_match.as_deref()
    }

    fn run_selector(&self) -> Option<&RunSelector> {
        self.run.as_ref()
    }

    fn location(&self, target: Option<&ResolvedElement>) -> ErrorLocation {
        ErrorLocation {
            part: target.map(|target| target.part.zip_entry_name().to_owned()),
            slide_id: Some(
                target
                    .map(|target| target.slide_id.clone())
                    .unwrap_or_else(|| self.slide_id.clone()),
            ),
            element_id: target.map(|target| target.element_id.clone()),
            operation_id: Some(self.operation_id.clone()),
            operation: Some("replace_text".to_owned()),
            ..ErrorLocation::default()
        }
    }

    fn missing_run_message(&self) -> &'static str {
        "replace_text notes target requires run."
    }

    fn newline_message(&self) -> &'static str {
        "replace_text notes target text must not contain line-break characters."
    }

    fn match_guard_message(&self) -> &'static str {
        "replace_text notes target match guard did not match current run text."
    }

    fn allow_default_run(&self) -> bool {
        false
    }
}

impl RunScopedTextOperation for ReplaceTableCellText {
    fn text(&self) -> &str {
        &self.text
    }

    fn current_text_match(&self) -> Option<&str> {
        self.current_text_match.as_deref()
    }

    fn run_selector(&self) -> Option<&RunSelector> {
        self.run.as_ref()
    }

    fn location(&self, target: Option<&ResolvedElement>) -> ErrorLocation {
        ReplaceTableCellText::location(self, target)
    }

    fn missing_run_message(&self) -> &'static str {
        "replace_text table cell target requires selector.run when the default first run is not selected."
    }

    fn newline_message(&self) -> &'static str {
        "replace_text table cell target text must not contain line-break characters."
    }

    fn match_guard_message(&self) -> &'static str {
        "replace_text table cell target match guard did not match current run text."
    }

    fn allow_default_run(&self) -> bool {
        true
    }
}

fn rewrite_text_body(
    document: &mut XmlDocument,
    operation: &ReplaceText,
    target: &ResolvedElement,
) -> Result<RewriteResult> {
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
    operation.validate_text_body(target, element, tx_body)?;
    let fit_warning = operation.fit_warning(element, tx_body);
    match operation.mode {
        ReplaceTextMode::WholeElement => {
            let replacement = replacement_text_body(tx_body, operation);
            let formatting_simplified = should_warn_formatting_simplified(tx_body, operation);
            element.children[tx_body_index] = XmlNode::Element(replacement);
            Ok(RewriteResult {
                formatting_simplified,
                fit_warning,
            })
        }
        ReplaceTextMode::RunScoped => {
            let tx_body =
                node_element_mut(&mut element.children[tx_body_index]).ok_or_else(|| {
                    Error::new(
                        ErrorCode::InternalError,
                        "Text body node is not an element.",
                    )
                })?;
            replace_run_scoped_text(tx_body, operation, target)?;
            Ok(RewriteResult {
                formatting_simplified: false,
                fit_warning,
            })
        }
    }
}

fn notes_body_tx_body(element: &XmlElement) -> Option<&XmlElement> {
    for child in element.children.iter().filter_map(XmlNode::as_element) {
        if child.name.local_name == "sp" {
            let has_body_placeholder = first_descendant(child, "ph").is_some_and(|ph| {
                ph.attributes.iter().any(|attribute| {
                    attribute.name.local_name == "type" && attribute.value == "body"
                })
            });
            if has_body_placeholder && let Some(tx_body) = child_element(child, "txBody") {
                return Some(tx_body);
            }
        }
        if let Some(descendant) = notes_body_tx_body(child) {
            return Some(descendant);
        }
    }
    None
}

fn notes_body_tx_body_mut(element: &mut XmlElement) -> Option<&mut XmlElement> {
    let body_shape_index = element.children.iter().position(|child| {
        child.as_element().is_some_and(|child| {
            child.name.local_name == "sp"
                && first_descendant(child, "ph").is_some_and(|ph| {
                    ph.attributes.iter().any(|attribute| {
                        attribute.name.local_name == "type" && attribute.value == "body"
                    })
                })
                && child_element(child, "txBody").is_some()
        })
    });
    if let Some(index) = body_shape_index {
        let shape = node_element_mut(&mut element.children[index])?;
        return child_element_mut(shape, "txBody");
    }

    for child in element.children.iter_mut().filter_map(node_element_mut) {
        if let Some(descendant) = notes_body_tx_body_mut(child) {
            return Some(descendant);
        }
    }
    None
}

struct RewriteResult {
    formatting_simplified: bool,
    fit_warning: Option<serde_json::Value>,
}

fn selected_run_text(
    tx_body: &XmlElement,
    run: &RunSelector,
    operation: &impl RunScopedTextOperation,
    target: &ResolvedElement,
) -> Result<String> {
    let indices = run_indices(tx_body, run, operation, target)?;
    let paragraph = paragraph_at(tx_body, run.paragraph_index, operation, target)?;
    let mut text = String::new();
    let first_index = indices[0];
    let last_index = *indices
        .last()
        .ok_or_else(|| Error::new(ErrorCode::InternalError, "Run range is empty."))?;
    for node in &paragraph.children[first_index..=last_index] {
        let Some(element) = node.as_element() else {
            continue;
        };
        match element.name.local_name.as_str() {
            "r" => text.push_str(&run_text(element)),
            "br" => text.push('\n'),
            _ => {}
        }
    }
    Ok(text)
}

fn replace_run_scoped_text(
    tx_body: &mut XmlElement,
    operation: &impl RunScopedTextOperation,
    target: &ResolvedElement,
) -> Result<()> {
    let default_run = RunSelector {
        paragraph_index: 0,
        run_index: 0,
        run_end_index: None,
        text_hash: None,
    };
    let run = operation
        .run_selector()
        .or_else(|| operation.allow_default_run().then_some(&default_run))
        .ok_or_else(|| {
            Error::new(ErrorCode::InvalidInput, operation.missing_run_message())
                .with_location(operation.location(Some(target)))
        })?;
    {
        let paragraph = paragraph_at_mut(tx_body, run.paragraph_index, operation, target)?;
        apply_paragraph_style(paragraph, operation.run_style());
    }
    let indices = run_indices(tx_body, run, operation, target)?;
    let first_index = indices[0];
    let last_index = *indices
        .last()
        .ok_or_else(|| Error::new(ErrorCode::InternalError, "Run range is empty."))?;
    let paragraph = paragraph_at_mut(tx_body, run.paragraph_index, operation, target)?;
    let mut remove_indices = indices.iter().skip(1).copied().collect::<Vec<_>>();
    remove_indices.extend((first_index + 1..last_index).filter(|index| {
        paragraph.children[*index]
            .as_element()
            .is_some_and(|child| child.name.local_name == "br")
    }));
    remove_indices.sort_unstable();
    remove_indices.dedup();
    for remove_index in remove_indices.into_iter().rev() {
        paragraph.children.remove(remove_index);
    }
    let first_run = paragraph.children[first_index]
        .as_element()
        .ok_or_else(|| Error::new(ErrorCode::InternalError, "Run node is not an element."))?
        .clone();
    let replacement_nodes = run_scoped_replacement_nodes(
        &first_run,
        operation.text(),
        operation.run_style(),
        operation,
        target,
    )?;
    paragraph.children[first_index] = replacement_nodes[0].clone();
    for (offset, node) in replacement_nodes.into_iter().enumerate().skip(1) {
        paragraph.children.insert(first_index + offset, node);
    }
    Ok(())
}

fn run_indices(
    tx_body: &XmlElement,
    run: &RunSelector,
    operation: &impl RunScopedTextOperation,
    target: &ResolvedElement,
) -> Result<Vec<usize>> {
    let end_index = run.run_end_index.unwrap_or(run.run_index);
    if end_index < run.run_index {
        return Err(Error::new(
            ErrorCode::InvalidInput,
            "selector.run run_end_index must be greater than or equal to run_index.",
        )
        .with_location(operation.location(Some(target))));
    }
    let paragraph = paragraph_at(tx_body, run.paragraph_index, operation, target)?;
    let start = usize::try_from(run.run_index).map_err(|_| {
        Error::new(
            ErrorCode::InvalidInput,
            "selector.run run_index is too large for this platform.",
        )
        .with_location(operation.location(Some(target)))
    })?;
    let end = usize::try_from(end_index).map_err(|_| {
        Error::new(
            ErrorCode::InvalidInput,
            "selector.run run_end_index is too large for this platform.",
        )
        .with_location(operation.location(Some(target)))
    })?;
    let run_child_indices = paragraph
        .children
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            node.as_element()
                .is_some_and(|child| child.name.local_name == "r")
                .then_some(index)
        })
        .collect::<Vec<_>>();
    if end >= run_child_indices.len() {
        return Err(Error::new(
            ErrorCode::SelectorNotFound,
            "selector.run resolved to a missing run.",
        )
        .with_location(operation.location(Some(target))));
    }
    Ok(run_child_indices[start..=end].to_vec())
}

fn paragraph_at<'a>(
    tx_body: &'a XmlElement,
    paragraph_index: u32,
    operation: &impl RunScopedTextOperation,
    target: &ResolvedElement,
) -> Result<&'a XmlElement> {
    let index = usize::try_from(paragraph_index).map_err(|_| {
        Error::new(
            ErrorCode::InvalidInput,
            "selector.run paragraph_index is too large for this platform.",
        )
        .with_location(operation.location(Some(target)))
    })?;
    tx_body
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .filter(|child| child.name.local_name == "p")
        .nth(index)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                "selector.run resolved to a missing paragraph.",
            )
            .with_location(operation.location(Some(target)))
        })
}

fn paragraph_at_mut<'a>(
    tx_body: &'a mut XmlElement,
    paragraph_index: u32,
    operation: &impl RunScopedTextOperation,
    target: &ResolvedElement,
) -> Result<&'a mut XmlElement> {
    let index = usize::try_from(paragraph_index).map_err(|_| {
        Error::new(
            ErrorCode::InvalidInput,
            "selector.run paragraph_index is too large for this platform.",
        )
        .with_location(operation.location(Some(target)))
    })?;
    tx_body
        .children
        .iter_mut()
        .filter_map(node_element_mut)
        .filter(|child| child.name.local_name == "p")
        .nth(index)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                "selector.run resolved to a missing paragraph.",
            )
            .with_location(operation.location(Some(target)))
        })
}

fn run_text(run: &XmlElement) -> String {
    run.children
        .iter()
        .filter_map(XmlNode::as_element)
        .filter(|child| child.name.local_name == "t")
        .flat_map(|text| text.children.iter())
        .filter_map(|node| match node {
            XmlNode::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>()
}

fn replace_run_text(
    run: &mut XmlElement,
    text: &str,
    operation: &impl RunScopedTextOperation,
    target: &ResolvedElement,
) -> Result<()> {
    let text_element = run
        .children
        .iter_mut()
        .filter_map(node_element_mut)
        .find(|child| child.name.local_name == "t")
        .ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                "Selected run does not contain literal a:t text.",
            )
            .with_location(operation.location(Some(target)))
        })?;
    text_element.children = vec![XmlNode::Text(text.to_owned())];
    Ok(())
}

fn run_scoped_replacement_nodes(
    run_template: &XmlElement,
    text: &str,
    run_style: Option<&TextRunStyle>,
    operation: &impl RunScopedTextOperation,
    target: &ResolvedElement,
) -> Result<Vec<XmlNode>> {
    let mut nodes = Vec::new();
    for (index, segment) in text.split('\u{000B}').enumerate() {
        if index > 0 {
            nodes.push(node(element("a:br", &[], Vec::new())));
        }
        let mut run = run_template.clone();
        replace_run_text(&mut run, segment, operation, target)?;
        apply_run_style(&mut run, run_style);
        nodes.push(node(run));
    }
    Ok(nodes)
}

fn validate_run_scoped_text_body(
    operation: &impl RunScopedTextOperation,
    target: &ResolvedElement,
    tx_body: &XmlElement,
) -> Result<()> {
    if operation.text().contains(['\n', '\r']) {
        return Err(
            Error::new(ErrorCode::InvalidInput, operation.newline_message())
                .with_location(operation.location(Some(target))),
        );
    }
    validate_xml_text(operation.text(), true)
        .map_err(|error| error.with_location(operation.location(Some(target))))?;
    let default_run = RunSelector {
        paragraph_index: 0,
        run_index: 0,
        run_end_index: None,
        text_hash: None,
    };
    let run = operation
        .run_selector()
        .or_else(|| operation.allow_default_run().then_some(&default_run))
        .ok_or_else(|| {
            Error::new(ErrorCode::InvalidInput, operation.missing_run_message())
                .with_location(operation.location(Some(target)))
        })?;
    let current = selected_run_text(tx_body, run, operation, target)?;
    if let Some(expected) = operation.current_text_match()
        && current != expected
    {
        return Err(Error::new(
            ErrorCode::SelectorGuardFailed,
            operation.match_guard_message(),
        )
        .with_location(operation.location(Some(target))));
    }
    if let Some(expected_hash) = &run.text_hash {
        let actual_hash = text_hash::text_hash(&current);
        if actual_hash != *expected_hash {
            return Err(Error::new(
                ErrorCode::SelectorGuardFailed,
                "selector.run text_hash guard did not match current run text.",
            )
            .with_location(operation.location(Some(target))));
        }
    }
    validate_run_style("replace_text.run_style", operation.run_style())
        .map_err(|error| error.with_location(operation.location(Some(target))))?;
    Ok(())
}

fn validate_xml_text(text: &str, allow_soft_break: bool) -> Result<()> {
    if let Some(character) = text.chars().find(|character| {
        !(is_xml_char(*character) || allow_soft_break && *character == '\u{000B}')
    }) {
        return Err(Error::new(
            ErrorCode::InvalidInput,
            format!(
                "replace_text text contains XML 1.0 illegal character U+{:04X}.",
                u32::from(character)
            ),
        ));
    }
    Ok(())
}

fn apply_paragraph_style(paragraph: &mut XmlElement, style: Option<&TextRunStyle>) {
    let Some(align) = style.and_then(|style| style.align) else {
        return;
    };
    let p_pr = ensure_child_element_at_front(paragraph, "pPr", "a:pPr");
    set_attribute(p_pr, "algn", align_value(align));
}

fn apply_run_style(run: &mut XmlElement, style: Option<&TextRunStyle>) {
    let Some(style) = style else {
        return;
    };
    let r_pr = ensure_child_element_at_front(run, "rPr", "a:rPr");
    if let Some(font_size) = style.font_size_pt {
        set_attribute(r_pr, "sz", &(font_size * 100).to_string());
    }
    if let Some(bold) = style.bold {
        set_attribute(r_pr, "b", bool_value(bold));
    }
    if let Some(italic) = style.italic {
        set_attribute(r_pr, "i", bool_value(italic));
    }
    if let Some(color) = &style.color_hex {
        upsert_single_child(
            r_pr,
            "solidFill",
            element(
                "a:solidFill",
                &[],
                vec![node(element("a:srgbClr", &[("val", color)], Vec::new()))],
            ),
        );
    }
    if let Some(font_family) = &style.font_family {
        upsert_single_child(
            r_pr,
            "latin",
            element("a:latin", &[("typeface", font_family)], Vec::new()),
        );
    }
}

fn table_cell<'a>(
    table: &'a XmlElement,
    row: u32,
    col: u32,
    operation: &ReplaceTableCellText,
    target: &ResolvedElement,
) -> Result<&'a XmlElement> {
    let row_index = usize::try_from(row).map_err(|_| {
        Error::new(
            ErrorCode::InvalidInput,
            "replace_text cell.row is too large for this platform.",
        )
        .with_location(operation.location(Some(target)))
    })?;
    let col_index = usize::try_from(col).map_err(|_| {
        Error::new(
            ErrorCode::InvalidInput,
            "replace_text cell.col is too large for this platform.",
        )
        .with_location(operation.location(Some(target)))
    })?;
    let row_element = table
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .filter(|child| child.name.local_name == "tr")
        .nth(row_index)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                "replace_text cell.row resolved to a missing table row.",
            )
            .with_location(operation.location(Some(target)))
        })?;
    row_element
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .filter(|child| child.name.local_name == "tc")
        .nth(col_index)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                "replace_text cell.col resolved to a missing table cell.",
            )
            .with_location(operation.location(Some(target)))
        })
}

fn table_cell_mut<'a>(
    table: &'a mut XmlElement,
    row: u32,
    col: u32,
    operation: &ReplaceTableCellText,
    target: &ResolvedElement,
) -> Result<&'a mut XmlElement> {
    let row_index = usize::try_from(row).map_err(|_| {
        Error::new(
            ErrorCode::InvalidInput,
            "replace_text cell.row is too large for this platform.",
        )
        .with_location(operation.location(Some(target)))
    })?;
    let col_index = usize::try_from(col).map_err(|_| {
        Error::new(
            ErrorCode::InvalidInput,
            "replace_text cell.col is too large for this platform.",
        )
        .with_location(operation.location(Some(target)))
    })?;
    let row_element = table
        .children
        .iter_mut()
        .filter_map(node_element_mut)
        .filter(|child| child.name.local_name == "tr")
        .nth(row_index)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                "replace_text cell.row resolved to a missing table row.",
            )
            .with_location(operation.location(Some(target)))
        })?;
    row_element
        .children
        .iter_mut()
        .filter_map(node_element_mut)
        .filter(|child| child.name.local_name == "tc")
        .nth(col_index)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                "replace_text cell.col resolved to a missing table cell.",
            )
            .with_location(operation.location(Some(target)))
        })
}

fn reject_merged_cell(
    cell: &XmlElement,
    operation: &ReplaceTableCellText,
    target: &ResolvedElement,
) -> Result<()> {
    if attr_u32_gt_one(cell, "gridSpan")
        || attr_u32_gt_one(cell, "rowSpan")
        || attr_present(cell, "vMerge")
        || child_element(cell, "vMerge").is_some()
    {
        return Err(Error::new(
            ErrorCode::UnsupportedEdit,
            "replace_text does not support merged or spanned table cells.",
        )
        .with_location(operation.location(Some(target))));
    }
    Ok(())
}

fn consult_table_style_read_model(
    package: &Package,
    table: &XmlElement,
    target: &ResolvedTableCell,
) {
    let catalog = TableStyleCatalog::from_package(package);
    consult_table_style_catalog(catalog.as_ref(), table, target);
}

fn consult_table_style_catalog(
    catalog: Option<&TableStyleCatalog>,
    table: &XmlElement,
    target: &ResolvedTableCell,
) {
    let Some(catalog) = catalog else {
        return;
    };
    let Some(tbl_pr) = child_element(table, "tblPr") else {
        return;
    };
    let properties = TableProperties::from_tbl_pr(tbl_pr);
    let row_count = child_elements(table, "tr").count();
    let col_count = child_element(table, "tblGrid")
        .map(|tbl_grid| child_elements(tbl_grid, "gridCol").count())
        .unwrap_or_else(|| {
            child_elements(table, "tr")
                .next()
                .map(|row| child_elements(row, "tc").count())
                .unwrap_or_default()
        });
    let Ok(row) = usize::try_from(target.row) else {
        return;
    };
    let Ok(col) = usize::try_from(target.col) else {
        return;
    };
    let _defaults = catalog.resolve_cell_defaults(&properties, row, col, row_count, col_count);
}

fn attr_present(element: &XmlElement, local_name: &str) -> bool {
    element
        .attributes
        .iter()
        .any(|attr| attr.name.local_name == local_name)
}

fn attr_value<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attr| attr.name.local_name == local_name)
        .map(|attr| attr.value.as_str())
}

fn attr_i64(element: &XmlElement, local_name: &str) -> Option<i64> {
    attr_value(element, local_name)?.parse().ok()
}

fn attr_u32_gt_one(element: &XmlElement, local_name: &str) -> bool {
    element
        .attributes
        .iter()
        .find(|attr| attr.name.local_name == local_name)
        .and_then(|attr| attr.value.parse::<u32>().ok())
        .is_some_and(|value| value > 1)
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
            first_run_properties(existing).map(style_run_properties_for_rewrite)
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
        return first_run_properties(existing).is_some()
            || run_count(existing) > 1
            || contains_rich_text_construct(existing);
    }
    run_count(existing) > 1 || contains_rich_text_construct(existing)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FitStatus {
    Fits,
    Overflow,
}

#[derive(Clone, Debug)]
struct FitEstimate {
    status: FitStatus,
    confidence: &'static str,
    estimated_lines: u32,
    available_height_emu: i64,
    scale_needed: f64,
    suggested_font_size_pt: u32,
    reason: Option<&'static str>,
}

impl FitEstimate {
    fn to_json(&self) -> serde_json::Value {
        json!({
            "status": match self.status {
                FitStatus::Fits => "fits",
                FitStatus::Overflow => "overflow",
            },
            "confidence": self.confidence,
            "estimated_lines": self.estimated_lines,
            "available_height_emu": self.available_height_emu,
            "scale_needed": self.scale_needed,
            "suggested_font_size_pt": self.suggested_font_size_pt,
            "reason": self.reason,
        })
    }
}

fn estimate_text_fit(
    element: &XmlElement,
    tx_body: &XmlElement,
    replacement_text: &str,
) -> FitEstimate {
    const EMU_PER_PT: f64 = 12_700.0;
    let bounds = element_bounds(element);
    let body_pr = child_element(tx_body, "bodyPr");
    let inset_l = body_pr
        .and_then(|body_pr| attr_i64(body_pr, "lIns"))
        .unwrap_or(91_440);
    let inset_r = body_pr
        .and_then(|body_pr| attr_i64(body_pr, "rIns"))
        .unwrap_or(91_440);
    let inset_t = body_pr
        .and_then(|body_pr| attr_i64(body_pr, "tIns"))
        .unwrap_or(45_720);
    let inset_b = body_pr
        .and_then(|body_pr| attr_i64(body_pr, "bIns"))
        .unwrap_or(45_720);
    let font_size = first_font_size_pt(tx_body).unwrap_or_else(|| fallback_font_size_pt(element));
    let Some((cx, cy)) = bounds else {
        return FitEstimate {
            status: FitStatus::Fits,
            confidence: "low",
            estimated_lines: 0,
            available_height_emu: 0,
            scale_needed: 1.0,
            suggested_font_size_pt: font_size,
            reason: Some("missing_bounds"),
        };
    };
    let available_width = cx.saturating_sub(inset_l).saturating_sub(inset_r).max(1) as f64;
    let available_height = cy.saturating_sub(inset_t).saturating_sub(inset_b).max(0);
    let glyph_width = f64::from(font_size) * EMU_PER_PT * average_glyph_em(replacement_text);
    let line_height = f64::from(font_size) * EMU_PER_PT * 1.2;
    let estimated_lines = estimate_lines(replacement_text, available_width, glyph_width);
    let needed_height = f64::from(estimated_lines) * line_height;
    let scale_needed = if needed_height <= 0.0 {
        1.0
    } else {
        ((available_height as f64) / needed_height).min(1.0)
    };
    let status = if needed_height > available_height as f64 {
        FitStatus::Overflow
    } else {
        FitStatus::Fits
    };
    FitEstimate {
        status,
        confidence: confidence(replacement_text, first_font_size_pt(tx_body).is_some()),
        estimated_lines,
        available_height_emu: available_height,
        scale_needed,
        suggested_font_size_pt: suggested_font_size(font_size, scale_needed),
        reason: (status == FitStatus::Overflow).then_some("estimated_text_height_exceeds_box"),
    }
}

fn element_bounds(element: &XmlElement) -> Option<(i64, i64)> {
    let xfrm = child_element(element, "spPr")
        .or_else(|| child_element(element, "grpSpPr"))
        .and_then(|properties| child_element(properties, "xfrm"))
        .or_else(|| child_element(element, "xfrm"))?;
    let ext = child_element(xfrm, "ext")?;
    Some((attr_i64(ext, "cx")?, attr_i64(ext, "cy")?))
}

fn first_font_size_pt(element: &XmlElement) -> Option<u32> {
    let run_properties = first_run_properties(element)?;
    let sz = attr_i64(run_properties, "sz")?;
    u32::try_from(sz / 100).ok().filter(|size| *size > 0)
}

fn fallback_font_size_pt(element: &XmlElement) -> u32 {
    match first_descendant(element, "ph").and_then(|placeholder| attr_value(placeholder, "type")) {
        Some("title" | "ctrTitle") => 32,
        Some("subTitle") => 24,
        Some("dt" | "ftr" | "sldNum" | "hdr") => 12,
        _ => 18,
    }
}

fn average_glyph_em(text: &str) -> f64 {
    if text.chars().any(is_cjk) { 0.85 } else { 0.55 }
}

fn estimate_lines(text: &str, available_width: f64, glyph_width: f64) -> u32 {
    let chars_per_line = (available_width / glyph_width.max(1.0)).floor().max(1.0) as usize;
    let mut lines = 0_u32;
    for paragraph in text.split('\n') {
        let chars = paragraph.chars().count().max(1);
        let paragraph_lines = chars.div_ceil(chars_per_line);
        lines = lines.saturating_add(u32::try_from(paragraph_lines).unwrap_or(u32::MAX));
    }
    lines.max(1)
}

fn suggested_font_size(font_size: u32, scale_needed: f64) -> u32 {
    ((f64::from(font_size) * scale_needed).floor() as u32).max(1)
}

fn confidence(text: &str, direct_font_size: bool) -> &'static str {
    if direct_font_size && !text.chars().any(is_complex_script) {
        "medium"
    } else {
        "low"
    }
}

fn is_cjk(character: char) -> bool {
    matches!(
        character as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0x3040..=0x30FF | 0xAC00..=0xD7AF
    )
}

fn is_complex_script(character: char) -> bool {
    matches!(
        character as u32,
        0x0590..=0x08FF | 0x0900..=0x0D7F | 0x1780..=0x18AF
    )
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

fn style_run_properties_for_rewrite(run_properties: &XmlElement) -> XmlElement {
    let mut run_properties = run_properties.clone();
    drop_rich_text_constructs(&mut run_properties);
    run_properties
}

fn drop_rich_text_constructs(element: &mut XmlElement) {
    element.children.retain_mut(|child| {
        let Some(child_element) = node_element_mut(child) else {
            return true;
        };
        if matches!(
            child_element.name.local_name.as_str(),
            "fld" | "hlinkClick" | "hlinkMouseOver" | "br"
        ) {
            return false;
        }
        drop_rich_text_constructs(child_element);
        true
    });
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
    element.children.iter().any(|child| match child {
        XmlNode::Element(child) => {
            matches!(
                child.name.local_name.as_str(),
                "pPr" | "fld" | "hlinkClick" | "hlinkMouseOver" | "br"
            ) || contains_literal_text_line_break(child)
                || contains_rich_text_construct(child)
        }
        XmlNode::Text(_)
        | XmlNode::CData(_)
        | XmlNode::Comment(_)
        | XmlNode::ProcessingInstruction(_)
        | XmlNode::DocType(_)
        | XmlNode::GeneralRef(_) => false,
    })
}

fn contains_literal_text_line_break(element: &XmlElement) -> bool {
    if element.name.local_name == "t" {
        return element.children.iter().any(|child| match child {
            XmlNode::Text(text) | XmlNode::CData(text) => {
                text.contains('\n') || text.contains('\r')
            }
            XmlNode::Element(child) => contains_literal_text_line_break(child),
            XmlNode::Comment(_)
            | XmlNode::ProcessingInstruction(_)
            | XmlNode::DocType(_)
            | XmlNode::GeneralRef(_) => false,
        });
    }
    element
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .any(contains_literal_text_line_break)
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

fn child_element<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a XmlElement> {
    element
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .find(|child| child.name.local_name == local_name)
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

fn ensure_child_element_at_front<'a>(
    parent: &'a mut XmlElement,
    local_name: &str,
    raw_name: &str,
) -> &'a mut XmlElement {
    if let Some(index) = parent.children.iter().position(|node| {
        node.as_element()
            .is_some_and(|child| child.name.local_name == local_name)
    }) {
        return node_element_mut(&mut parent.children[index])
            .expect("position already confirmed element");
    }
    parent
        .children
        .insert(0, node(element(raw_name, &[], Vec::new())));
    node_element_mut(&mut parent.children[0]).expect("inserted node is an element")
}

fn set_attribute(element: &mut XmlElement, local_name: &str, value: &str) {
    if let Some(attribute) = element
        .attributes
        .iter_mut()
        .find(|attribute| attribute.name.local_name == local_name)
    {
        attribute.value = value.to_owned();
    } else {
        element.attributes.push(XmlAttribute {
            name: QualifiedName::from_raw(local_name),
            value: value.to_owned(),
            namespace_declaration: false,
        });
    }
}

fn upsert_single_child(parent: &mut XmlElement, local_name: &str, replacement: XmlElement) {
    parent.children.retain(|node| {
        node.as_element()
            .is_none_or(|child| child.name.local_name != local_name)
    });
    let insertion_index = parent
        .children
        .iter()
        .position(|node| {
            node.as_element()
                .is_some_and(|child| child.name.local_name == "t")
        })
        .unwrap_or(parent.children.len());
    parent.children.insert(insertion_index, node(replacement));
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
    use pptx_compose_core::{opc::part_name::PartName, pptx::ids::ElementKind};

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
        element_id: "slide-1:shape-1".to_owned(),
        kind: ElementKind::TextBox,
        part: slide_part.clone(),
        sp_tree_path: vec![1],
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
        allow_formatting_simplification: false,
        run: None,
        run_style: None,
        fit_policy: None,
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
        allow_formatting_simplification: true,
        ..operation
    };
    let error = guarded
        .validate(&package, &target)
        .expect_err("stale match guard fails");
    assert_eq!(error.code(), ErrorCode::SelectorGuardFailed);
}
