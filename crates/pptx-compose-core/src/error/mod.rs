#![allow(clippy::module_name_repetitions)]

use std::{error, fmt, io};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorCode {
    InvalidInput,
    UnsafePath,
    DuplicatePart,
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
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::UnsafePath => "unsafe_path",
            Self::DuplicatePart => "duplicate_part",
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

#[derive(Debug)]
pub struct Error {
    code: ErrorCode,
    message: String,
    source: Option<Box<dyn error::Error + Send + Sync + 'static>>,
}

impl Error {
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            source: None,
        }
    }

    #[must_use]
    pub fn with_source(
        code: ErrorCode,
        message: impl Into<String>,
        source: impl error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    #[must_use]
    pub fn unsupported_package(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::UnsupportedPackage, message)
    }

    #[must_use]
    pub fn unsafe_path(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::UnsafePath, message)
    }

    #[must_use]
    pub fn duplicate_part(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::DuplicatePart, message)
    }

    #[must_use]
    pub fn resource_limit_exceeded(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ResourceLimitExceeded, message)
    }

    #[must_use]
    pub fn parse_error(
        message: impl Into<String>,
        source: impl error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::with_source(ErrorCode::ParseError, message, source)
    }

    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.message)
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn error::Error + 'static))
    }
}

impl From<io::Error> for Error {
    fn from(source: io::Error) -> Self {
        Self::parse_error("Could not read package bytes.", source)
    }
}
