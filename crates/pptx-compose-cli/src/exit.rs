use pptx_compose::core::error::ErrorCode;

use crate::{CliError, InvalidInputCause};

pub(crate) const SUCCESS: i32 = 0;
pub(crate) const USAGE: i32 = 1;
pub(crate) const INPUT_FILE_UNREADABLE: i32 = 2;
pub(crate) const PATH_PERMISSION: i32 = 3;
pub(crate) const PARSE_OPEN: i32 = 10;
pub(crate) const UNSUPPORTED_PACKAGE: i32 = 11;
pub(crate) const RESOURCE_LIMIT: i32 = 12;
pub(crate) const PATCH_INVALID: i32 = 20;
pub(crate) const STALE_PATCH: i32 = 21;
pub(crate) const SELECTOR_RESOLUTION: i32 = 22;
pub(crate) const MEDIA_RESOLUTION: i32 = 23;
pub(crate) const UNSUPPORTED_OPERATION: i32 = 24;
pub(crate) const VALIDATION_FAILURE: i32 = 30;
pub(crate) const WRITE_FAILURE: i32 = 40;
pub(crate) const INTERNAL_ERROR: i32 = 50;

pub(crate) const fn exit_code_for(err: &CliError) -> i32 {
    match err.code() {
        ErrorCode::InvalidInput => match err.invalid_input_cause() {
            Some(InvalidInputCause::CliArgument) => USAGE,
            Some(InvalidInputCause::InputPath) => INPUT_FILE_UNREADABLE,
            Some(InvalidInputCause::PatchSchema) => PATCH_INVALID,
            None => INTERNAL_ERROR,
        },
        ErrorCode::UnsafePath | ErrorCode::PermissionDenied => PATH_PERMISSION,
        ErrorCode::ParseError | ErrorCode::MalformedXml => PARSE_OPEN,
        ErrorCode::UnsupportedPackage => UNSUPPORTED_PACKAGE,
        ErrorCode::ResourceLimitExceeded => RESOURCE_LIMIT,
        ErrorCode::InvalidBounds => PATCH_INVALID,
        ErrorCode::StalePatch => STALE_PATCH,
        ErrorCode::SelectorNotFound
        | ErrorCode::SelectorAmbiguous
        | ErrorCode::SelectorGuardFailed => SELECTOR_RESOLUTION,
        ErrorCode::MissingMediaRef
        | ErrorCode::MediaChecksumMismatch
        | ErrorCode::UnsupportedMediaType => MEDIA_RESOLUTION,
        ErrorCode::UnsupportedEdit => UNSUPPORTED_OPERATION,
        ErrorCode::ValidationFailed => VALIDATION_FAILURE,
        ErrorCode::WriteFailed => WRITE_FAILURE,
        ErrorCode::InternalError => INTERNAL_ERROR,
    }
}

#[cfg(test)]
#[test]
fn maps_071_table() {
    use pptx_compose::core::error::Error;

    let cases = [
        (
            CliError::invalid_input(
                InvalidInputCause::CliArgument,
                "bad command-line flag value",
            ),
            USAGE,
        ),
        (
            CliError::invalid_input(InvalidInputCause::InputPath, "input path is unreadable"),
            INPUT_FILE_UNREADABLE,
        ),
        (
            CliError::invalid_input(InvalidInputCause::PatchSchema, "patch failed schema"),
            PATCH_INVALID,
        ),
        (
            CliError::new(ErrorCode::UnsafePath, "unsafe path"),
            PATH_PERMISSION,
        ),
        (
            CliError::new(ErrorCode::PermissionDenied, "permission denied"),
            PATH_PERMISSION,
        ),
        (
            CliError::new(ErrorCode::ParseError, "parse failed"),
            PARSE_OPEN,
        ),
        (
            CliError::new(ErrorCode::MalformedXml, "malformed xml"),
            PARSE_OPEN,
        ),
        (
            CliError::new(ErrorCode::UnsupportedPackage, "unsupported package"),
            UNSUPPORTED_PACKAGE,
        ),
        (
            CliError::new(ErrorCode::ResourceLimitExceeded, "resource limit"),
            RESOURCE_LIMIT,
        ),
        (
            CliError::new(ErrorCode::InvalidBounds, "invalid bounds"),
            PATCH_INVALID,
        ),
        (
            CliError::new(ErrorCode::StalePatch, "stale patch"),
            STALE_PATCH,
        ),
        (
            CliError::new(ErrorCode::SelectorNotFound, "selector not found"),
            SELECTOR_RESOLUTION,
        ),
        (
            CliError::new(ErrorCode::SelectorAmbiguous, "selector ambiguous"),
            SELECTOR_RESOLUTION,
        ),
        (
            CliError::new(ErrorCode::SelectorGuardFailed, "selector guard failed"),
            SELECTOR_RESOLUTION,
        ),
        (
            CliError::new(ErrorCode::MissingMediaRef, "missing media ref"),
            MEDIA_RESOLUTION,
        ),
        (
            CliError::new(ErrorCode::MediaChecksumMismatch, "media checksum mismatch"),
            MEDIA_RESOLUTION,
        ),
        (
            CliError::new(ErrorCode::UnsupportedMediaType, "unsupported media type"),
            MEDIA_RESOLUTION,
        ),
        (
            CliError::new(ErrorCode::UnsupportedEdit, "unsupported edit"),
            UNSUPPORTED_OPERATION,
        ),
        (
            CliError::new(ErrorCode::ValidationFailed, "validation failed"),
            VALIDATION_FAILURE,
        ),
        (
            CliError::new(ErrorCode::WriteFailed, "write failed"),
            WRITE_FAILURE,
        ),
        (
            CliError::new(ErrorCode::InternalError, "internal error"),
            INTERNAL_ERROR,
        ),
    ];

    for (error, expected_exit) in &cases {
        assert_eq!(exit_code_for(error), *expected_exit, "{error}");
    }

    for code in ErrorCode::ALL {
        let covered = cases.iter().any(|(error, _)| error.code() == code);
        assert!(
            covered,
            "{} is missing from the exit map test",
            code.as_str()
        );
    }

    let malformed = CliError::new(ErrorCode::MalformedXml, "malformed xml");
    assert_eq!(exit_code_for(&malformed), PARSE_OPEN);

    let invalid_without_cause = CliError::from_error(Error::new(
        ErrorCode::InvalidInput,
        "invalid input without CLI sub-cause",
    ));
    assert_eq!(exit_code_for(&invalid_without_cause), INTERNAL_ERROR);
}
