use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const JOURNAL_SCHEMA: &str = "pptx-compose.journal.v1";
pub const JOURNAL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransactionJournal {
    #[serde(default = "journal_schema")]
    pub schema: String,
    #[serde(default = "journal_version")]
    pub version: u32,
    pub transaction_id: String,
    pub document_id: String,
    pub base_revision: u32,
    pub status: JournalStatus,
    pub operations: Vec<String>,
    pub changed_parts: Vec<JournalChangedPart>,
    pub validation_report: String,
    pub output_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JournalStatus {
    Pending,
    Applied,
    Failed,
    RolledBack,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JournalChangedPart {
    pub part: String,
    pub before_checksum: String,
    pub after_checksum: String,
}

fn journal_schema() -> String {
    JOURNAL_SCHEMA.to_owned()
}

const fn journal_version() -> u32 {
    JOURNAL_VERSION
}
