use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SEMANTIC_DIFF_SCHEMA: &str = "pptx-compose.semantic_diff.v1";
pub const SEMANTIC_DIFF_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticDiff {
    #[serde(default = "semantic_diff_schema")]
    pub schema: String,
    #[serde(default = "semantic_diff_version")]
    pub version: u32,
    pub changes: Vec<DiffChange>,
    pub changed_parts: Vec<ChangedPart>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DiffChange {
    TextReplaced {
        operation_id: String,
        element_id: String,
        before: Value,
        after: Value,
    },
    RelationshipAdded {
        part: String,
        relationship_id: String,
        target: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangedPart {
    pub part: String,
    pub change_kind: PartChangeKind,
    pub before_checksum: String,
    pub after_checksum: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PartChangeKind {
    ModifiedXml,
    AddedPart,
    RemovedPart,
    ModifiedBinary,
    ModifiedRelationships,
    ModifiedContentTypes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PreviewLevel {
    /// Always available: structured semantic JSON diff.
    JsonDiff,
    /// Always available: package part and checksum changes.
    PackageDiff,
    /// Always available on dry-run: validation status before write.
    ValidationPreview,
    /// Optional debug preview; not required for V1 rendering.
    XmlExcerpt,
    /// Optional preview requiring an external renderer.
    SlideThumbnail,
}

impl PreviewLevel {
    #[must_use]
    pub const fn is_always_available(self) -> bool {
        matches!(
            self,
            Self::JsonDiff | Self::PackageDiff | Self::ValidationPreview
        )
    }
}

fn semantic_diff_schema() -> String {
    SEMANTIC_DIFF_SCHEMA.to_owned()
}

const fn semantic_diff_version() -> u32 {
    SEMANTIC_DIFF_VERSION
}

#[cfg(test)]
#[test]
fn roundtrips_045_examples() {
    assert_roundtrip::<SemanticDiff>(
        r#"{
          "schema": "pptx-compose.semantic_diff.v1",
          "version": 1,
          "changes": [
            {
              "kind": "text_replaced",
              "operation_id": "op-1",
              "element_id": "slide-1:shape-4",
              "before": { "text": "Quarterly Results" },
              "after": { "text": "Updated title" }
            },
            {
              "kind": "relationship_added",
              "part": "ppt/slides/_rels/slide1.xml.rels",
              "relationship_id": "rId8",
              "target": "../media/image7.png"
            }
          ],
          "changed_parts": [
            {
              "part": "ppt/slides/slide1.xml",
              "change_kind": "modified_xml",
              "before_checksum": "sha256:...",
              "after_checksum": "sha256:..."
            }
          ]
        }"#,
    );

    assert_roundtrip::<crate::journal::TransactionJournal>(
        r#"{
          "schema": "pptx-compose.journal.v1",
          "version": 1,
          "transaction_id": "txn_...",
          "document_id": "sha256:old",
          "base_revision": 1,
          "status": "applied",
          "operations": ["op-1"],
          "changed_parts": [
            { "part": "ppt/slides/slide1.xml", "before_checksum": "sha256:...", "after_checksum": "sha256:..." }
          ],
          "validation_report": "validation-report-id-or-inline",
          "output_path": "output.pptx"
        }"#,
    );
}

#[cfg(test)]
fn assert_roundtrip<T>(json: &str)
where
    T: serde::de::DeserializeOwned + Serialize,
{
    let input: Value = serde_json::from_str(json).expect("045 example is valid JSON");
    let envelope: T = serde_json::from_value(input.clone()).expect("045 example deserializes");
    let output = serde_json::to_value(envelope).expect("045 example re-serializes");

    assert_eq!(output, input);
}
