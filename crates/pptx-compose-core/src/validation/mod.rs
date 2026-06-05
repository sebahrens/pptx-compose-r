pub mod code;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use code::{FINDING_REGISTRY, FindingCategory, FindingCode, Severity};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub severity: Severity,
    pub category: FindingCategory,
    pub code: FindingCode,
    pub message: String,
    pub blocking: bool,
    pub location: Value,
    pub suggested_action: Option<String>,
}

impl Finding {
    /// Finding severity starts at the registry default and may only be escalated.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        code: FindingCode,
        message: impl Into<String>,
        blocking: bool,
        location: Value,
        suggested_action: Option<String>,
    ) -> Self {
        let (category, severity) = code.default_entry();

        Self {
            id: id.into(),
            severity,
            category,
            code,
            message: message.into(),
            blocking,
            location,
            suggested_action,
        }
    }
}
