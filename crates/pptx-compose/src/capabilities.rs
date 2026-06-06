use serde::Serialize;

use crate::{
    core::{error::ErrorCode, zip::limits::ResourceLimits},
    edit::{
        media_inputs::{MEDIA_MANIFEST_SCHEMA, MEDIA_MANIFEST_VERSION, limits::MediaLimits},
        patch::{PATCH_SCHEMA, PATCH_VERSION},
    },
    json::schema_versions::{
        AGENT_VIEW_SCHEMA, AGENT_VIEW_VERSION, CAPABILITIES_SCHEMA, CAPABILITIES_VERSION,
        ERROR_SCHEMA, ERROR_VERSION, PATCH_REPORT_SCHEMA, PATCH_REPORT_VERSION, RESULT_SCHEMA,
        RESULT_VERSION, VALIDATION_REPORT_SCHEMA, VALIDATION_REPORT_VERSION,
    },
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilitiesDocument {
    pub schema: String,
    pub version: u32,
    pub status: CapabilitiesStatus,
    pub package: PackageInfo,
    pub commands: Vec<CommandCapability>,
    pub supported_operations: Vec<OperationCapability>,
    pub schemas: Vec<SchemaCapability>,
    pub exit_codes: Vec<ExitCodeCapability>,
    pub raw_xml_enabled: bool,
    pub default_limits: DefaultLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitiesStatus {
    Success,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandCapability {
    pub name: String,
    pub read_only: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationCapability {
    pub op: String,
    pub dry_run: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaCapability {
    pub name: String,
    pub schema: String,
    pub version: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExitCodeCapability {
    pub exit: i32,
    pub meaning: String,
    pub error_codes: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DefaultLimits {
    pub max_compressed_package_bytes: u64,
    pub max_uncompressed_package_bytes: u64,
    pub max_part_count: usize,
    pub max_single_part_uncompressed_bytes: u64,
    pub max_media_part_bytes: u64,
    pub max_per_entry_compression_ratio: u64,
    pub max_xml_depth: usize,
    pub max_xml_node_count: u64,
    pub max_media_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilitiesOptions {
    pub package_name: String,
    pub package_version: String,
    pub raw_xml_enabled: bool,
}

impl CapabilitiesOptions {
    #[must_use]
    pub fn new(package_name: impl Into<String>, package_version: impl Into<String>) -> Self {
        Self {
            package_name: package_name.into(),
            package_version: package_version.into(),
            raw_xml_enabled: false,
        }
    }

    #[must_use]
    pub const fn with_raw_xml_enabled(mut self, raw_xml_enabled: bool) -> Self {
        self.raw_xml_enabled = raw_xml_enabled;
        self
    }
}

#[must_use]
pub fn capabilities(options: CapabilitiesOptions) -> CapabilitiesDocument {
    let resource_limits = ResourceLimits::default();
    let media_limits = MediaLimits::default();
    CapabilitiesDocument {
        schema: CAPABILITIES_SCHEMA.to_owned(),
        version: CAPABILITIES_VERSION,
        status: CapabilitiesStatus::Success,
        package: PackageInfo {
            name: options.package_name,
            version: options.package_version,
        },
        commands: command_capabilities(),
        supported_operations: operation_capabilities(),
        schemas: schema_capabilities(),
        exit_codes: exit_code_capabilities(),
        raw_xml_enabled: options.raw_xml_enabled,
        default_limits: DefaultLimits {
            max_compressed_package_bytes: resource_limits.max_compressed_package_bytes,
            max_uncompressed_package_bytes: resource_limits.max_uncompressed_package_bytes,
            max_part_count: resource_limits.max_part_count,
            max_single_part_uncompressed_bytes: resource_limits.max_single_part_uncompressed_bytes,
            max_media_part_bytes: resource_limits.max_media_part_bytes,
            max_per_entry_compression_ratio: resource_limits.max_per_entry_compression_ratio,
            max_xml_depth: resource_limits.max_xml_depth,
            max_xml_node_count: resource_limits.max_xml_node_count,
            max_media_bytes: media_limits.max_media_bytes,
        },
    }
}

fn command_capabilities() -> Vec<CommandCapability> {
    [
        ("capabilities", true),
        ("inspect", true),
        ("find-text", true),
        ("validate", true),
        ("apply --dry-run", true),
        ("apply", false),
        ("media list", true),
        ("media get", true),
        ("schema", true),
    ]
    .into_iter()
    .map(|(name, read_only)| CommandCapability {
        name: name.to_owned(),
        read_only,
    })
    .collect()
}

fn operation_capabilities() -> Vec<OperationCapability> {
    [
        "replace_text",
        "add_text_box",
        "move_resize_element",
        "set_alt_text",
        "add_image",
        "replace_image",
    ]
    .into_iter()
    .map(|op| OperationCapability {
        op: op.to_owned(),
        dry_run: true,
    })
    .collect()
}

fn schema_capabilities() -> Vec<SchemaCapability> {
    [
        ("capabilities-v1", CAPABILITIES_SCHEMA, CAPABILITIES_VERSION),
        ("agent-view-v1", AGENT_VIEW_SCHEMA, AGENT_VIEW_VERSION),
        ("patch-v1", PATCH_SCHEMA, PATCH_VERSION),
        (
            "media-manifest-v1",
            MEDIA_MANIFEST_SCHEMA,
            MEDIA_MANIFEST_VERSION,
        ),
        ("patch-report-v1", PATCH_REPORT_SCHEMA, PATCH_REPORT_VERSION),
        (
            "validation-report-v1",
            VALIDATION_REPORT_SCHEMA,
            VALIDATION_REPORT_VERSION,
        ),
        ("result-v1", RESULT_SCHEMA, RESULT_VERSION),
        ("error-v1", ERROR_SCHEMA, ERROR_VERSION),
    ]
    .into_iter()
    .map(|(name, schema, version)| SchemaCapability {
        name: name.to_owned(),
        schema: schema.to_owned(),
        version,
    })
    .collect()
}

fn exit_code_capabilities() -> Vec<ExitCodeCapability> {
    [
        (0, "success", &[][..]),
        (
            1,
            "command-line usage error",
            &[ErrorCode::InvalidInput][..],
        ),
        (
            2,
            "input file not found or unreadable",
            &[ErrorCode::InvalidInput][..],
        ),
        (
            3,
            "unsafe path or permission violation",
            &[ErrorCode::UnsafePath, ErrorCode::PermissionDenied][..],
        ),
        (
            10,
            "parse or open failure",
            &[ErrorCode::ParseError, ErrorCode::MalformedXml][..],
        ),
        (
            11,
            "unsupported or encrypted package",
            &[ErrorCode::UnsupportedPackage][..],
        ),
        (
            12,
            "resource limit exceeded",
            &[ErrorCode::ResourceLimitExceeded][..],
        ),
        (
            20,
            "patch invalid",
            &[ErrorCode::InvalidInput, ErrorCode::InvalidBounds][..],
        ),
        (
            21,
            "stale document or revision",
            &[ErrorCode::StalePatch][..],
        ),
        (
            22,
            "selector resolution failure",
            &[
                ErrorCode::SelectorNotFound,
                ErrorCode::SelectorAmbiguous,
                ErrorCode::SelectorGuardFailed,
            ][..],
        ),
        (
            23,
            "media resolution failure",
            &[
                ErrorCode::MissingMediaRef,
                ErrorCode::MediaChecksumMismatch,
                ErrorCode::UnsupportedMediaType,
            ][..],
        ),
        (
            24,
            "unsupported operation",
            &[ErrorCode::UnsupportedEdit][..],
        ),
        (30, "validation failure", &[ErrorCode::ValidationFailed][..]),
        (40, "write failure", &[ErrorCode::WriteFailed][..]),
        (50, "internal error", &[ErrorCode::InternalError][..]),
    ]
    .into_iter()
    .map(|(exit, meaning, error_codes)| ExitCodeCapability {
        exit,
        meaning: meaning.to_owned(),
        error_codes: error_codes
            .iter()
            .map(|code| code.as_str().to_owned())
            .collect(),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_enumerates_v1_agent_contract() {
        let document = capabilities(
            CapabilitiesOptions::new("pptx-compose", "0.1.0").with_raw_xml_enabled(true),
        );

        assert_eq!(document.schema, "pptx-compose.capabilities.v1");
        assert_eq!(document.version, 1);
        assert!(document.raw_xml_enabled);
        assert!(document.schemas.iter().any(|schema| {
            schema.name == "patch-v1" && schema.schema == "pptx-compose.patch.v1"
        }));
        assert!(
            document
                .supported_operations
                .iter()
                .any(|op| op.op == "replace_text")
        );
        assert!(
            document
                .supported_operations
                .iter()
                .any(|op| op.op == "add_image")
        );
        assert!(
            document
                .exit_codes
                .iter()
                .any(|entry| { entry.exit == 24 && entry.error_codes == ["unsupported_edit"] })
        );
    }
}
