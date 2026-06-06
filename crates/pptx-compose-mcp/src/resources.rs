use std::{fmt, str::FromStr};

use pptx_compose::{
    core::error::{Error, ErrorCode},
    edit::patch::{PATCH_SCHEMA, PATCH_VERSION, Patch},
    json::{
        agent_view::views::{ViewMode, ViewRequest, build_view, package_from_pptx_bytes},
        schema_versions::{AGENT_VIEW_SCHEMA, ERROR_SCHEMA, PATCH_REPORT_SCHEMA},
        schemas::{ErrorEnvelope, JsonError, PatchReport, agent_view_json_schema},
    },
};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};

use crate::sessions::SessionStore;

pub type McpError = Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceUri {
    SessionSummary {
        session_id: String,
    },
    SessionSlides {
        session_id: String,
        cursor: Option<String>,
        limit: Option<u32>,
    },
    SessionSlide {
        session_id: String,
        slide_id: String,
    },
    SessionElement {
        session_id: String,
        element_id: String,
    },
    SessionMediaMetadata {
        session_id: String,
        media_id: String,
    },
    SessionLatestValidation {
        session_id: String,
    },
    Schema {
        name: String,
        version: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceDescriptor {
    pub uri: String,
    pub name: String,
    pub title: String,
    pub description: String,
    pub mime_type: String,
    pub read_only: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceContent {
    pub uri: String,
    pub mime_type: String,
    pub content: Value,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ResourceRegistry;

impl ResourceRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    #[must_use]
    pub fn list_resources(&self, session_id: Option<&str>) -> Vec<ResourceDescriptor> {
        let mut resources = schema_resource_descriptors();
        if let Some(session_id) = session_id {
            resources.extend(session_resource_descriptors(session_id));
        }
        resources
    }

    pub async fn read_resource(
        &self,
        uri: &ResourceUri,
        sessions: &SessionStore,
    ) -> Result<ResourceContent, McpError> {
        let content = match uri {
            ResourceUri::SessionSummary { session_id } => session_view(
                sessions,
                session_id,
                ViewMode::DeckSummary,
                None,
                None,
                None,
                None,
            )?,
            ResourceUri::SessionSlides {
                session_id,
                cursor,
                limit,
            } => session_view(
                sessions,
                session_id,
                ViewMode::SlidePage,
                None,
                None,
                cursor.clone(),
                *limit,
            )?,
            ResourceUri::SessionSlide {
                session_id,
                slide_id,
            } => session_view(
                sessions,
                session_id,
                ViewMode::SlideDetail,
                Some(slide_id.clone()),
                None,
                None,
                None,
            )?,
            ResourceUri::SessionElement {
                session_id,
                element_id,
            } => session_view(
                sessions,
                session_id,
                ViewMode::ElementDetail,
                None,
                Some(element_id.clone()),
                None,
                None,
            )?,
            ResourceUri::SessionMediaMetadata {
                session_id,
                media_id,
            } => {
                let session = sessions.get(session_id)?;
                let handle = session.media.get(media_id).ok_or_else(|| {
                    Error::new(
                        ErrorCode::InvalidInput,
                        format!("Media handle {media_id} does not exist in session {session_id}."),
                    )
                })?;
                json!(handle)
            }
            ResourceUri::SessionLatestValidation { session_id } => session_view(
                sessions,
                session_id,
                ViewMode::ValidationReport,
                None,
                None,
                None,
                None,
            )?,
            ResourceUri::Schema { name, version } => schema_resource(name, version)?,
        };

        Ok(ResourceContent {
            uri: uri.to_string(),
            mime_type: "application/json".to_owned(),
            content,
        })
    }
}

impl FromStr for ResourceUri {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let (path, query) = input
            .strip_prefix("pptx://")
            .ok_or_else(|| invalid_uri(input, "Resource URI must start with pptx://."))?
            .split_once('?')
            .map_or(
                (input.strip_prefix("pptx://").unwrap_or_default(), None),
                |(path, query)| (path, Some(query)),
            );
        let segments = path.split('/').collect::<Vec<_>>();

        match segments.as_slice() {
            ["sessions", session_id, "summary"] if !session_id.is_empty() => {
                Ok(Self::SessionSummary {
                    session_id: (*session_id).to_owned(),
                })
            }
            ["sessions", session_id, "slides"] if !session_id.is_empty() => {
                let query = Query::parse(query)?;
                Ok(Self::SessionSlides {
                    session_id: (*session_id).to_owned(),
                    cursor: query.cursor,
                    limit: query.limit,
                })
            }
            ["sessions", session_id, "slides", slide_id]
                if !session_id.is_empty() && !slide_id.is_empty() =>
            {
                Ok(Self::SessionSlide {
                    session_id: (*session_id).to_owned(),
                    slide_id: (*slide_id).to_owned(),
                })
            }
            ["sessions", session_id, "elements", element_id]
                if !session_id.is_empty() && !element_id.is_empty() =>
            {
                Ok(Self::SessionElement {
                    session_id: (*session_id).to_owned(),
                    element_id: (*element_id).to_owned(),
                })
            }
            ["sessions", session_id, "media", media_id, "metadata"]
                if !session_id.is_empty() && !media_id.is_empty() =>
            {
                Ok(Self::SessionMediaMetadata {
                    session_id: (*session_id).to_owned(),
                    media_id: (*media_id).to_owned(),
                })
            }
            ["sessions", session_id, "validation", "latest"] if !session_id.is_empty() => {
                Ok(Self::SessionLatestValidation {
                    session_id: (*session_id).to_owned(),
                })
            }
            ["schemas", name, version] if !name.is_empty() && !version.is_empty() => {
                Ok(Self::Schema {
                    name: (*name).to_owned(),
                    version: (*version).to_owned(),
                })
            }
            _ => Err(invalid_uri(
                input,
                "Resource URI does not match a V1 pptx resource.",
            )),
        }
    }
}

impl fmt::Display for ResourceUri {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SessionSummary { session_id } => {
                write!(formatter, "pptx://sessions/{session_id}/summary")
            }
            Self::SessionSlides {
                session_id,
                cursor,
                limit,
            } => {
                write!(formatter, "pptx://sessions/{session_id}/slides")?;
                write_query(formatter, cursor.as_deref(), *limit)
            }
            Self::SessionSlide {
                session_id,
                slide_id,
            } => write!(formatter, "pptx://sessions/{session_id}/slides/{slide_id}"),
            Self::SessionElement {
                session_id,
                element_id,
            } => write!(
                formatter,
                "pptx://sessions/{session_id}/elements/{element_id}"
            ),
            Self::SessionMediaMetadata {
                session_id,
                media_id,
            } => write!(
                formatter,
                "pptx://sessions/{session_id}/media/{media_id}/metadata"
            ),
            Self::SessionLatestValidation { session_id } => {
                write!(formatter, "pptx://sessions/{session_id}/validation/latest")
            }
            Self::Schema { name, version } => {
                write!(formatter, "pptx://schemas/{name}/{version}")
            }
        }
    }
}

#[derive(Default)]
struct Query {
    cursor: Option<String>,
    limit: Option<u32>,
}

impl Query {
    fn parse(query: Option<&str>) -> Result<Self, Error> {
        let mut output = Self::default();
        for pair in query.into_iter().flat_map(|query| query.split('&')) {
            if pair.is_empty() {
                continue;
            }
            let (key, value) = pair.split_once('=').ok_or_else(|| {
                invalid_uri(pair, "Resource query parameters must use key=value syntax.")
            })?;
            match key {
                "cursor" if !value.is_empty() => output.cursor = Some(value.to_owned()),
                "limit" if !value.is_empty() => {
                    output.limit = Some(value.parse::<u32>().map_err(|source| {
                        Error::with_source(
                            ErrorCode::InvalidInput,
                            "Resource limit query parameter must be an unsigned integer.",
                            source,
                        )
                    })?);
                }
                _ => {
                    return Err(invalid_uri(
                        pair,
                        "Resource query parameter is not supported for this URI.",
                    ));
                }
            }
        }
        Ok(output)
    }
}

fn session_view(
    sessions: &SessionStore,
    session_id: &str,
    mode: ViewMode,
    slide_id: Option<String>,
    element_id: Option<String>,
    cursor: Option<String>,
    limit: Option<u32>,
) -> Result<Value, Error> {
    let session = sessions.get(session_id)?;
    let package = package_from_pptx_bytes(session.package.source_bytes()).map_err(json_error)?;
    build_view(
        &package,
        ViewRequest {
            mode,
            slide_id,
            element_id,
            cursor,
            limit,
        },
    )
    .map_err(json_error)
}

fn schema_resource(name: &str, version: &str) -> Result<Value, Error> {
    if version != "v1" {
        return Err(invalid_uri(
            version,
            "Only v1 schema resources are available.",
        ));
    }

    match name {
        "agent-view" => agent_view_json_schema().map_err(json_error),
        "patch" => schema_value::<Patch>(PATCH_SCHEMA),
        "patch-report" => schema_value::<PatchReport>(PATCH_REPORT_SCHEMA),
        "error" => schema_value::<ErrorEnvelope>(ERROR_SCHEMA),
        _ => Err(invalid_uri(name, "Unknown schema resource name.")),
    }
}

fn schema_value<T: JsonSchema>(id: &str) -> Result<Value, Error> {
    let schema = schemars::schema_for!(T);
    let mut value = serde_json::to_value(schema).map_err(|source| {
        Error::with_source(
            ErrorCode::InternalError,
            "Could not serialize JSON schema resource.",
            source,
        )
    })?;
    if let Some(object) = value.as_object_mut() {
        object.insert("$id".to_owned(), Value::String(id.to_owned()));
    }
    Ok(value)
}

fn schema_resource_descriptors() -> Vec<ResourceDescriptor> {
    [
        ("agent-view", AGENT_VIEW_SCHEMA),
        ("patch", PATCH_SCHEMA),
        ("patch-report", PATCH_REPORT_SCHEMA),
        ("error", ERROR_SCHEMA),
    ]
    .into_iter()
    .map(|(name, schema)| ResourceDescriptor {
        uri: ResourceUri::Schema {
            name: name.to_owned(),
            version: format!("v{PATCH_VERSION}"),
        }
        .to_string(),
        name: format!("{name}-schema"),
        title: format!("{name} v1 schema"),
        description: format!("Draft 2020-12 JSON schema for {schema}."),
        mime_type: "application/schema+json".to_owned(),
        read_only: true,
    })
    .collect()
}

fn session_resource_descriptors(session_id: &str) -> Vec<ResourceDescriptor> {
    [
        (
            ResourceUri::SessionSummary {
                session_id: session_id.to_owned(),
            },
            "session-summary",
            "Session summary",
            "Bounded deck summary for the open PPTX session.",
        ),
        (
            ResourceUri::SessionSlides {
                session_id: session_id.to_owned(),
                cursor: None,
                limit: None,
            },
            "session-slides",
            "Session slides",
            "Paginated slide summaries for the open PPTX session.",
        ),
        (
            ResourceUri::SessionLatestValidation {
                session_id: session_id.to_owned(),
            },
            "session-latest-validation",
            "Latest validation",
            "Latest validation report for the open PPTX session.",
        ),
    ]
    .into_iter()
    .map(|(uri, name, title, description)| ResourceDescriptor {
        uri: uri.to_string(),
        name: name.to_owned(),
        title: title.to_owned(),
        description: description.to_owned(),
        mime_type: "application/json".to_owned(),
        read_only: true,
    })
    .collect()
}

fn write_query(
    formatter: &mut fmt::Formatter<'_>,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> fmt::Result {
    let mut separator = '?';
    if let Some(cursor) = cursor {
        write!(formatter, "{separator}cursor={cursor}")?;
        separator = '&';
    }
    if let Some(limit) = limit {
        write!(formatter, "{separator}limit={limit}")?;
    }
    Ok(())
}

fn json_error(error: JsonError) -> Error {
    Error::new(
        ErrorCode::InvalidInput,
        format!("Could not build resource view: {error:?}."),
    )
}

fn invalid_uri(input: &str, message: &str) -> Error {
    Error::new(
        ErrorCode::InvalidInput,
        format!("{message} Input: {input}."),
    )
}
