use pptx_compose_core::{
    error::{Error, ErrorCode, ErrorLocation, Result},
    opc::package::Package,
    xml::{
        document::{QualifiedName, XmlDocument, XmlElement, XmlNode},
        namespaces::{NamespaceBinding, NamespaceTable},
        parser::parse_document,
        writer::{WriteMode, WriteOptions, write_document},
    },
};
use pptx_compose_json::schemas::OperationTarget;

use crate::{
    operations::ResolvedCoreProperties,
    patch::{DocumentMetadataFields, PatchEffects, SetDocumentMetadataOperation},
};

const CP_NS: &str = "http://schemas.openxmlformats.org/package/2006/metadata/core-properties";
const DC_NS: &str = "http://purl.org/dc/elements/1.1/";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetDocumentMetadata {
    pub operation_id: String,
    pub current_value_match: Option<DocumentMetadataFields>,
    pub metadata: DocumentMetadataFields,
}

impl From<&SetDocumentMetadataOperation> for SetDocumentMetadata {
    fn from(operation: &SetDocumentMetadataOperation) -> Self {
        Self {
            operation_id: operation.operation_id.clone(),
            current_value_match: operation.current_value_match.clone(),
            metadata: operation.metadata.clone(),
        }
    }
}

impl SetDocumentMetadata {
    pub fn validate(&self, package: &Package, target: &ResolvedCoreProperties) -> Result<()> {
        self.validate_fields(None)?;
        let part = package.parts().get(&target.part).ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                format!("Core-properties part {} was not found.", target.part),
            )
            .with_location(self.location(Some(target)))
        })?;
        let document = parse_document(part.bytes()).map_err(|source| {
            Error::with_source(
                source.code(),
                format!("Could not parse core-properties part {}.", target.part),
                source,
            )
            .with_location(self.location(Some(target)))
        })?;
        self.validate_fields(Some(&document))?;
        Ok(())
    }

    pub fn apply(
        &self,
        package: &mut Package,
        target: &ResolvedCoreProperties,
    ) -> Result<PatchEffects> {
        self.validate_fields(None)?;
        let part_name = target.part.clone();
        let part = package.parts_mut().get_mut(&part_name).ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                format!("Core-properties part {part_name} was not found."),
            )
            .with_location(self.location(Some(target)))
        })?;
        let mut document = parse_document(part.bytes()).map_err(|source| {
            Error::with_source(
                source.code(),
                format!("Could not parse core-properties part {part_name}."),
                source,
            )
            .with_location(self.location(Some(target)))
        })?;
        self.validate_fields(Some(&document))?;

        rewrite_metadata(&mut document, &self.metadata, target, self)?;
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
                slide_id: String::new(),
                element_id: String::new(),
                part: part_name.zip_entry_name().to_owned(),
            }),
            created_element_ids: Vec::new(),
            warnings: Vec::new(),
        })
    }

    fn validate_fields(&self, document: Option<&XmlDocument>) -> Result<()> {
        if self.metadata.is_empty() {
            return Err(Error::new(
                ErrorCode::InvalidInput,
                "set_document_metadata requires at least one metadata field.",
            )
            .with_location(self.location(None)));
        }

        let Some(document) = document else {
            return Ok(());
        };
        if let Some(expected) = &self.current_value_match {
            for field in field_specs() {
                let Some(expected_value) = (field.value)(expected) else {
                    continue;
                };
                let actual = read_field(document, field.local_name, field.namespace);
                if actual.as_deref() != Some(expected_value) {
                    return Err(Error::new(
                        ErrorCode::SelectorGuardFailed,
                        format!(
                            "Metadata match guard {} did not match the current value.",
                            field.name
                        ),
                    )
                    .with_location(ErrorLocation {
                        operation_id: Some(self.operation_id.clone()),
                        operation: Some("set_document_metadata".to_owned()),
                        expected: Some(expected_value.to_owned()),
                        actual,
                        ..ErrorLocation::default()
                    }));
                }
            }
        }
        Ok(())
    }

    fn location(&self, target: Option<&ResolvedCoreProperties>) -> ErrorLocation {
        ErrorLocation {
            part: target.map(|target| target.part.zip_entry_name().to_owned()),
            operation_id: Some(self.operation_id.clone()),
            operation: Some("set_document_metadata".to_owned()),
            ..ErrorLocation::default()
        }
    }
}

struct FieldSpec {
    name: &'static str,
    namespace: &'static str,
    preferred_prefix: &'static str,
    local_name: &'static str,
    value: fn(&DocumentMetadataFields) -> Option<&String>,
}

fn field_specs() -> [FieldSpec; 4] {
    [
        FieldSpec {
            name: "title",
            namespace: DC_NS,
            preferred_prefix: "dc",
            local_name: "title",
            value: |metadata| metadata.title.as_ref(),
        },
        FieldSpec {
            name: "subject",
            namespace: DC_NS,
            preferred_prefix: "dc",
            local_name: "subject",
            value: |metadata| metadata.subject.as_ref(),
        },
        FieldSpec {
            name: "creator",
            namespace: DC_NS,
            preferred_prefix: "dc",
            local_name: "creator",
            value: |metadata| metadata.creator.as_ref(),
        },
        FieldSpec {
            name: "keywords",
            namespace: CP_NS,
            preferred_prefix: "cp",
            local_name: "keywords",
            value: |metadata| metadata.keywords.as_ref(),
        },
    ]
}

fn rewrite_metadata(
    document: &mut XmlDocument,
    metadata: &DocumentMetadataFields,
    target: &ResolvedCoreProperties,
    operation: &SetDocumentMetadata,
) -> Result<()> {
    let root = document
        .nodes
        .iter_mut()
        .find_map(node_element_mut)
        .ok_or_else(|| {
            Error::malformed_xml("Core-properties XML does not contain a root element.")
                .with_location(operation.location(Some(target)))
        })?;
    if root.name.local_name != "coreProperties" {
        return Err(Error::malformed_xml(
            "Core-properties XML root element is not coreProperties.",
        )
        .with_location(operation.location(Some(target))));
    }

    for field in field_specs() {
        if let Some(value) = (field.value)(metadata) {
            set_field(root, &field, value);
        }
    }

    Ok(())
}

fn set_field(root: &mut XmlElement, field: &FieldSpec, value: &str) {
    let root_namespaces = root.namespaces.clone();
    if let Some(element) = root
        .children
        .iter_mut()
        .filter_map(node_element_mut)
        .find(|element| {
            element_matches(element, &root_namespaces, field.local_name, field.namespace)
        })
    {
        element.children.clear();
        element.children.push(XmlNode::Text(value.to_owned()));
        return;
    }

    let prefix = prefix_for_namespace(root, field.namespace, field.preferred_prefix);
    root.children.push(XmlNode::Element(XmlElement {
        name: QualifiedName::from_raw(format!("{prefix}:{}", field.local_name)),
        attributes: Vec::new(),
        namespaces: Default::default(),
        children: vec![XmlNode::Text(value.to_owned())],
    }));
}

fn read_field(document: &XmlDocument, local_name: &str, namespace: &str) -> Option<String> {
    let root = document.root_element()?;
    root.children
        .iter()
        .filter_map(XmlNode::as_element)
        .find(|element| element_matches(element, &root.namespaces, local_name, namespace))
        .map(text_content)
}

fn text_content(element: &XmlElement) -> String {
    element
        .children
        .iter()
        .filter_map(|child| match child {
            XmlNode::Text(text) | XmlNode::CData(text) => Some(text.as_str()),
            XmlNode::Element(_)
            | XmlNode::Comment(_)
            | XmlNode::ProcessingInstruction(_)
            | XmlNode::DocType(_)
            | XmlNode::GeneralRef(_) => None,
        })
        .collect()
}

fn element_matches(
    element: &XmlElement,
    root_namespaces: &NamespaceTable,
    local_name: &str,
    namespace: &str,
) -> bool {
    element.name.local_name == local_name
        && element.name.prefix.as_deref().and_then(|prefix| {
            element
                .namespaces
                .resolve_prefix(Some(prefix))
                .or_else(|| root_namespaces.resolve_prefix(Some(prefix)))
        }) == Some(namespace)
}

fn prefix_for_namespace(root: &mut XmlElement, namespace: &str, preferred_prefix: &str) -> String {
    if let Some(existing) = root
        .namespaces
        .bindings()
        .iter()
        .rev()
        .find(|binding| binding.uri == namespace)
        .and_then(|binding| binding.prefix.clone())
    {
        return existing;
    }

    root.namespaces
        .push(NamespaceBinding::prefixed(preferred_prefix, namespace));
    preferred_prefix.to_owned()
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

#[cfg(test)]
mod tests {
    use pptx_compose_core::{
        error::ErrorCode,
        opc::{package::Package, part_name::PartName},
        xml::{document::XmlElement, parser::parse_document},
    };

    use super::{DocumentMetadataFields, ResolvedCoreProperties, SetDocumentMetadata};

    #[test]
    fn sets_core_metadata_and_dirties_only_core_properties() {
        let core_part = part("docProps/core.xml");
        let app_part = part("docProps/app.xml");
        let mut package = Package::new();
        package
            .insert_zip_entry(
                "docProps/core.xml",
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/"><dc:title>Old</dc:title><dcterms:created>2026-01-01T00:00:00Z</dcterms:created></cp:coreProperties>"#.to_vec(),
            )
            .expect("core part inserted");
        package
            .insert_zip_entry("docProps/app.xml", b"<Properties/>".to_vec())
            .expect("app part inserted");
        let app_before = package
            .parts()
            .get(&app_part)
            .expect("app part exists")
            .bytes()
            .to_vec();
        let operation = SetDocumentMetadata {
            operation_id: "op-meta".to_owned(),
            current_value_match: Some(DocumentMetadataFields {
                title: Some("Old".to_owned()),
                ..DocumentMetadataFields::default()
            }),
            metadata: DocumentMetadataFields {
                title: Some("New".to_owned()),
                subject: Some("Board update".to_owned()),
                creator: Some("Research Team".to_owned()),
                keywords: Some("finance; q4".to_owned()),
            },
        };
        let target = ResolvedCoreProperties {
            part: core_part.clone(),
        };

        let effects = operation
            .apply(&mut package, &target)
            .expect("metadata applies");

        assert_eq!(effects.changed_parts, vec!["docProps/core.xml"]);
        assert!(package.dirty_parts().contains(&core_part));
        assert_eq!(package.dirty_parts().len(), 1);
        assert_eq!(
            package
                .parts()
                .get(&app_part)
                .expect("app part still exists")
                .bytes(),
            app_before
        );
        let document = parse_document(
            package
                .parts()
                .get(&core_part)
                .expect("core part still exists")
                .bytes(),
        )
        .expect("updated core part parses");
        let root = document.root_element().expect("root exists");
        assert_eq!(text(root, "title"), Some("New".to_owned()));
        assert_eq!(text(root, "subject"), Some("Board update".to_owned()));
        assert_eq!(text(root, "creator"), Some("Research Team".to_owned()));
        assert_eq!(text(root, "keywords"), Some("finance; q4".to_owned()));
        assert!(
            root.children
                .iter()
                .filter_map(|node| node.as_element())
                .any(|element| element.name.local_name == "created"
                    && element.name.prefix.as_deref() == Some("dcterms"))
        );
    }

    #[test]
    fn mismatched_current_value_guard_fails() {
        let core_part = part("docProps/core.xml");
        let mut package = Package::new();
        package
            .insert_zip_entry(
                "docProps/core.xml",
                br#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Actual</dc:title></cp:coreProperties>"#.to_vec(),
            )
            .expect("core part inserted");
        let operation = SetDocumentMetadata {
            operation_id: "op-meta".to_owned(),
            current_value_match: Some(DocumentMetadataFields {
                title: Some("Expected".to_owned()),
                ..DocumentMetadataFields::default()
            }),
            metadata: DocumentMetadataFields {
                title: Some("New".to_owned()),
                ..DocumentMetadataFields::default()
            },
        };
        let target = ResolvedCoreProperties { part: core_part };

        let error = operation
            .validate(&package, &target)
            .expect_err("guard mismatch fails");

        assert_eq!(error.code(), ErrorCode::SelectorGuardFailed);
    }

    fn part(name: &str) -> PartName {
        PartName::from_zip_entry(name).expect("valid part name")
    }

    fn text(root: &XmlElement, local_name: &str) -> Option<String> {
        root.children
            .iter()
            .filter_map(|node| node.as_element())
            .find(|element| element.name.local_name == local_name)
            .map(super::text_content)
    }
}
