use serde::{Deserialize, Serialize};

/// Canonical stable error-code vocabulary from specs/044.
///
/// Renaming a variant changes the serialized wire string and is a breaking API
/// change. Additions must be made in specs/044 first.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput,
    UnsafePath,
    ResourceLimitExceeded,
    UnsupportedPackage,
    UnsupportedEdit,
    UnsupportedMediaType,
    InvalidBounds,
    ParseError,
    ValidationFailed,
    StalePatch,
    SelectorNotFound,
    SelectorAmbiguous,
    SelectorGuardFailed,
    MissingMediaRef,
    MediaChecksumMismatch,
    PermissionDenied,
    WriteFailed,
    InternalError,
}

impl ErrorCode {
    pub const ALL: [Self; 18] = [
        Self::InvalidInput,
        Self::UnsafePath,
        Self::ResourceLimitExceeded,
        Self::UnsupportedPackage,
        Self::UnsupportedEdit,
        Self::UnsupportedMediaType,
        Self::InvalidBounds,
        Self::ParseError,
        Self::ValidationFailed,
        Self::StalePatch,
        Self::SelectorNotFound,
        Self::SelectorAmbiguous,
        Self::SelectorGuardFailed,
        Self::MissingMediaRef,
        Self::MediaChecksumMismatch,
        Self::PermissionDenied,
        Self::WriteFailed,
        Self::InternalError,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::UnsafePath => "unsafe_path",
            Self::ResourceLimitExceeded => "resource_limit_exceeded",
            Self::UnsupportedPackage => "unsupported_package",
            Self::UnsupportedEdit => "unsupported_edit",
            Self::UnsupportedMediaType => "unsupported_media_type",
            Self::InvalidBounds => "invalid_bounds",
            Self::ParseError => "parse_error",
            Self::ValidationFailed => "validation_failed",
            Self::StalePatch => "stale_patch",
            Self::SelectorNotFound => "selector_not_found",
            Self::SelectorAmbiguous => "selector_ambiguous",
            Self::SelectorGuardFailed => "selector_guard_failed",
            Self::MissingMediaRef => "missing_media_ref",
            Self::MediaChecksumMismatch => "media_checksum_mismatch",
            Self::PermissionDenied => "permission_denied",
            Self::WriteFailed => "write_failed",
            Self::InternalError => "internal_error",
        }
    }
}

#[test]
fn serializes_canonical_strings() {
    let expected = [
        (ErrorCode::InvalidInput, "invalid_input"),
        (ErrorCode::UnsafePath, "unsafe_path"),
        (ErrorCode::ResourceLimitExceeded, "resource_limit_exceeded"),
        (ErrorCode::UnsupportedPackage, "unsupported_package"),
        (ErrorCode::UnsupportedEdit, "unsupported_edit"),
        (ErrorCode::UnsupportedMediaType, "unsupported_media_type"),
        (ErrorCode::InvalidBounds, "invalid_bounds"),
        (ErrorCode::ParseError, "parse_error"),
        (ErrorCode::ValidationFailed, "validation_failed"),
        (ErrorCode::StalePatch, "stale_patch"),
        (ErrorCode::SelectorNotFound, "selector_not_found"),
        (ErrorCode::SelectorAmbiguous, "selector_ambiguous"),
        (ErrorCode::SelectorGuardFailed, "selector_guard_failed"),
        (ErrorCode::MissingMediaRef, "missing_media_ref"),
        (ErrorCode::MediaChecksumMismatch, "media_checksum_mismatch"),
        (ErrorCode::PermissionDenied, "permission_denied"),
        (ErrorCode::WriteFailed, "write_failed"),
        (ErrorCode::InternalError, "internal_error"),
    ];

    assert_eq!(ErrorCode::ALL.len(), 18);
    assert_eq!(ErrorCode::ALL, expected.map(|(code, _)| code));
    for (code, wire_string) in expected {
        assert_eq!(code.as_str(), wire_string);
        assert_eq!(
            serde_json::to_value(code).expect("error code serializes"),
            wire_string
        );
    }
}
