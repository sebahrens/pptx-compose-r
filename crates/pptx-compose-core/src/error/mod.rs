pub mod code;

use std::{error, fmt, io};

use serde::{Deserialize, Serialize};

pub use code::ErrorCode;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorSeverity {
    Info,
    Warning,
    Error,
    Fatal,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Input,
    Path,
    Resource,
    Package,
    Edit,
    Media,
    Bounds,
    Parse,
    Validation,
    Patch,
    Selector,
    Permission,
    Write,
    Internal,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ErrorLocation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub io_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zip_entry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slide_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<String>,
}

#[derive(Debug)]
pub struct Error {
    details: Box<ErrorDetails>,
    source: Option<Box<dyn error::Error + Send + Sync + 'static>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ErrorDetails {
    pub code: ErrorCode,
    pub message: String,
    pub severity: ErrorSeverity,
    pub category: ErrorCategory,
    pub retryable: bool,
    pub state_changed: bool,
    pub location: ErrorLocation,
    pub suggestions: Vec<String>,
}

impl Error {
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            details: Box::new(ErrorDetails {
                code,
                message: message.into(),
                severity: ErrorSeverity::Error,
                category: code.default_category(),
                retryable: false,
                state_changed: false,
                location: ErrorLocation::default(),
                suggestions: Vec::new(),
            }),
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
            details: Box::new(ErrorDetails {
                code,
                message: message.into(),
                severity: ErrorSeverity::Error,
                category: code.default_category(),
                retryable: false,
                state_changed: false,
                location: ErrorLocation::default(),
                suggestions: Vec::new(),
            }),
            source: Some(Box::new(source)),
        }
    }

    #[must_use]
    pub fn unsupported_package(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::UnsupportedPackage, message)
    }

    #[must_use]
    pub fn malformed_package(message: impl Into<String>) -> Self {
        Self::unsupported_package(message)
    }

    #[must_use]
    pub fn unsafe_path(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::UnsafePath, message)
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
    pub fn malformed_xml(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::MalformedXml, message)
    }

    #[must_use]
    pub fn malformed_xml_with_source(
        message: impl Into<String>,
        source: impl error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::with_source(ErrorCode::MalformedXml, message, source)
    }

    #[must_use]
    pub fn stale_revision(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::StalePatch, message)
            .with_suggestion("Inspect the deck again and regenerate the patch.")
    }

    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        self.details.code
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.details.message
    }

    #[must_use]
    pub fn details(&self) -> &ErrorDetails {
        &self.details
    }

    #[must_use]
    pub fn with_location(mut self, location: ErrorLocation) -> Self {
        self.details.location = location;
        self
    }

    #[must_use]
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.details.suggestions.push(suggestion.into());
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: {}",
            self.details.code.as_str(),
            self.details.message
        )
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn error::Error + 'static))
    }
}

impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.details.serialize(serializer)
    }
}

impl From<io::Error> for Error {
    fn from(source: io::Error) -> Self {
        Self::parse_error("Could not read package bytes.", source)
    }
}

impl ErrorCode {
    #[must_use]
    pub const fn default_category(self) -> ErrorCategory {
        match self {
            Self::InvalidInput => ErrorCategory::Input,
            Self::UnsafePath => ErrorCategory::Path,
            Self::ResourceLimitExceeded => ErrorCategory::Resource,
            Self::UnsupportedPackage => ErrorCategory::Package,
            Self::UnsupportedEdit => ErrorCategory::Edit,
            Self::UnsupportedMediaType => ErrorCategory::Media,
            Self::InvalidBounds => ErrorCategory::Bounds,
            Self::ParseError | Self::MalformedXml => ErrorCategory::Parse,
            Self::ValidationFailed => ErrorCategory::Validation,
            Self::StalePatch => ErrorCategory::Patch,
            Self::SelectorNotFound | Self::SelectorAmbiguous | Self::SelectorGuardFailed => {
                ErrorCategory::Selector
            }
            Self::MissingMediaRef | Self::MediaChecksumMismatch => ErrorCategory::Media,
            Self::PermissionDenied => ErrorCategory::Permission,
            Self::WriteFailed => ErrorCategory::Write,
            Self::InternalError => ErrorCategory::Internal,
        }
    }
}
