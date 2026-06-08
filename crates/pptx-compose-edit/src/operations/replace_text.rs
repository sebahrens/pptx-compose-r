use pptx_compose_core::{
    error::{Error, ErrorCode, ErrorLocation, Result},
    opc::package::Package,
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
        ResolvedElement, ResolvedNotesSlide, ResolvedTableCell, add_text_box::validate_style,
        is_real_shape_tree_child,
    },
    patch::{
        FormatPolicy, PatchEffects, ReplaceTextMode, ReplaceTextOperation, TextAlign, TextBoxStyle,
    },
    selectors::RunSelector,
};

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
    pub run_style: Option<TextBoxStyle>,
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
        self.validate_text_body(target, tx_body)
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

    fn validate_text_body(&self, target: &ResolvedElement, tx_body: &XmlElement) -> Result<()> {
        match self.mode {
            ReplaceTextMode::WholeElement => {
                self.validate_whole_element_text_body(target, tx_body)?;
                self.validate_whole_element_match(target, tx_body)
            }
            ReplaceTextMode::RunScoped => validate_run_scoped_text_body(self, target, tx_body),
        }
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
    fn run_style(&self) -> Option<&TextBoxStyle> {
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
        "run_scoped replace_text text must not contain newline characters."
    }

    fn match_guard_message(&self) -> &'static str {
        "replace_text match guard did not match current run text."
    }

    fn allow_default_run(&self) -> bool {
        false
    }

    fn run_style(&self) -> Option<&TextBoxStyle> {
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
        "replace_text notes target text must not contain newline characters."
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
        "replace_text table cell target text must not contain newline characters."
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
    operation.validate_text_body(target, tx_body)?;
    match operation.mode {
        ReplaceTextMode::WholeElement => {
            let replacement = replacement_text_body(tx_body, operation);
            let formatting_simplified = should_warn_formatting_simplified(tx_body, operation);
            element.children[tx_body_index] = XmlNode::Element(replacement);
            Ok(RewriteResult {
                formatting_simplified,
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
    for run_index in indices {
        let run_element = paragraph.children[run_index]
            .as_element()
            .ok_or_else(|| Error::new(ErrorCode::InternalError, "Run node is not an element."))?;
        text.push_str(&run_text(run_element));
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
    let paragraph = paragraph_at_mut(tx_body, run.paragraph_index, operation, target)?;
    for remove_index in indices.iter().skip(1).rev() {
        paragraph.children.remove(*remove_index);
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
    run_style: Option<&TextBoxStyle>,
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
    validate_style("replace_text.run_style", operation.run_style())
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

fn apply_paragraph_style(paragraph: &mut XmlElement, style: Option<&TextBoxStyle>) {
    let Some(align) = style.and_then(|style| style.align) else {
        return;
    };
    let p_pr = ensure_child_element_at_front(paragraph, "pPr", "a:pPr");
    set_attribute(p_pr, "algn", align_value(align));
}

fn apply_run_style(run: &mut XmlElement, style: Option<&TextBoxStyle>) {
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
