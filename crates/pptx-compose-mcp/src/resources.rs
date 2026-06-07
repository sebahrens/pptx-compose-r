use std::{fmt, str::FromStr};

use pptx_compose::{
    AgentViewOptions,
    capabilities::{CapabilitiesOptions, capabilities},
    core::error::{Error, ErrorCode},
    edit::patch::{PATCH_SCHEMA, PATCH_VERSION, patch_json_schema},
    json::{
        agent_view::views::ViewMode,
        schema_versions::{
            AGENT_VIEW_SCHEMA, AGENT_VIEW_VERSION, ERROR_SCHEMA, ERROR_VERSION,
            PATCH_REPORT_SCHEMA, PATCH_REPORT_VERSION,
        },
        schemas::{JsonError, agent_view_json_schema, error_json_schema, patch_report_json_schema},
    },
};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};

use crate::sessions::SessionStore;

pub type McpError = Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceUri {
    Capabilities,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceTemplateDescriptor {
    pub uri_template: String,
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
pub struct ResourceRegistry {
    raw_xml_enabled: bool,
}

impl ResourceRegistry {
    #[must_use]
    pub const fn new() -> Self {
        Self::with_raw_xml_enabled(false)
    }

    #[must_use]
    pub const fn with_raw_xml_enabled(raw_xml_enabled: bool) -> Self {
        Self { raw_xml_enabled }
    }

    #[must_use]
    pub fn list_resources(&self, session_id: Option<&str>) -> Vec<ResourceDescriptor> {
        let mut resources = vec![capabilities_resource_descriptor()];
        resources.extend(schema_resource_descriptors());
        if let Some(session_id) = session_id {
            resources.extend(session_resource_descriptors(session_id));
        }
        resources
    }

    #[must_use]
    pub fn list_resource_templates(&self) -> Vec<ResourceTemplateDescriptor> {
        session_resource_template_descriptors()
    }

    pub async fn read_resource(
        &self,
        uri: &ResourceUri,
        sessions: &SessionStore,
    ) -> Result<ResourceContent, McpError> {
        let content = match uri {
            ResourceUri::Capabilities => json!(capabilities(
                CapabilitiesOptions::new("pptx-compose-mcp", env!("CARGO_PKG_VERSION"))
                    .with_raw_xml_enabled(self.raw_xml_enabled),
            )),
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
            ResourceUri::SessionLatestValidation { session_id } => {
                session_latest_validation(sessions, session_id)?
            }
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
            ["capabilities", "v1"] => Ok(Self::Capabilities),
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
            Self::Capabilities => write!(formatter, "pptx://capabilities/v1"),
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

fn session_latest_validation(sessions: &SessionStore, session_id: &str) -> Result<Value, Error> {
    let session = sessions.get(session_id)?;
    if let Some(latest) = sessions.latest_validation(session_id)? {
        return Ok(json!({
            "status": "validated",
            "session_id": session_id,
            "revision": latest.revision,
            "validated_at": latest.validated_at,
            "source": latest.source,
            "report": latest.report
        }));
    }

    Ok(json!({
        "status": "not_yet_validated",
        "session_id": session_id,
        "revision": session.revision,
        "message": "Run pptx_validate for this session before reading validation/latest."
    }))
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
    session
        .package
        .to_agent_json_with_options(AgentViewOptions {
            mode,
            include_elements: false,
            slide_id,
            slide_ids: Vec::new(),
            element_id,
            cursor,
            limit,
        })
}

fn schema_resource(name: &str, version: &str) -> Result<Value, Error> {
    match name {
        "agent-view" => {
            require_schema_version(name, version, AGENT_VIEW_VERSION)?;
            agent_view_json_schema().map_err(json_error)
        }
        "patch" => {
            require_schema_version(name, version, PATCH_VERSION)?;
            patch_json_schema()
        }
        "patch-report" => {
            require_schema_version(name, version, PATCH_REPORT_VERSION)?;
            patch_report_json_schema().map_err(json_error)
        }
        "error" => {
            require_schema_version(name, version, ERROR_VERSION)?;
            error_json_schema().map_err(json_error)
        }
        _ => Err(invalid_uri(name, "Unknown schema resource name.")),
    }
}

fn require_schema_version(name: &str, version: &str, expected: u32) -> Result<(), Error> {
    let expected_version = schema_version(expected);
    if version == expected_version {
        Ok(())
    } else {
        Err(invalid_uri(
            version,
            &format!("Schema resource {name} is only available at {expected_version}."),
        ))
    }
}

fn schema_resource_descriptors() -> Vec<ResourceDescriptor> {
    [
        ("agent-view", AGENT_VIEW_SCHEMA, AGENT_VIEW_VERSION),
        ("patch", PATCH_SCHEMA, PATCH_VERSION),
        ("patch-report", PATCH_REPORT_SCHEMA, PATCH_REPORT_VERSION),
        ("error", ERROR_SCHEMA, ERROR_VERSION),
    ]
    .into_iter()
    .map(|(name, schema, version)| ResourceDescriptor {
        uri: ResourceUri::Schema {
            name: name.to_owned(),
            version: schema_version(version),
        }
        .to_string(),
        name: format!("{name}-schema"),
        title: format!("{name} v{version} schema"),
        description: format!("Draft 2020-12 JSON schema for {schema}."),
        mime_type: "application/schema+json".to_owned(),
        read_only: true,
    })
    .collect()
}

fn schema_version(version: u32) -> String {
    format!("v{version}")
}

fn capabilities_resource_descriptor() -> ResourceDescriptor {
    ResourceDescriptor {
        uri: ResourceUri::Capabilities.to_string(),
        name: "capabilities".to_owned(),
        title: "Capabilities".to_owned(),
        description: "Versioned machine-readable CLI and MCP capabilities for agents.".to_owned(),
        mime_type: "application/json".to_owned(),
        read_only: true,
    }
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

fn session_resource_template_descriptors() -> Vec<ResourceTemplateDescriptor> {
    [
        (
            "pptx://sessions/{session_id}/summary",
            "session-summary",
            "Session summary",
            "Bounded deck summary for an open PPTX session.",
        ),
        (
            "pptx://sessions/{session_id}/slides",
            "session-slides",
            "Session slides",
            "Paginated slide summaries for an open PPTX session.",
        ),
        (
            "pptx://sessions/{session_id}/slides/{slide_id}",
            "session-slide",
            "Session slide",
            "One slide view for an open PPTX session.",
        ),
        (
            "pptx://sessions/{session_id}/elements/{element_id}",
            "session-element",
            "Session element",
            "One element detail view for an open PPTX session.",
        ),
        (
            "pptx://sessions/{session_id}/media/{media_id}/metadata",
            "session-media-metadata",
            "Session media metadata",
            "Metadata for a staged media handle in an open PPTX session.",
        ),
        (
            "pptx://sessions/{session_id}/validation/latest",
            "session-latest-validation",
            "Latest validation",
            "Latest validation report for an open PPTX session.",
        ),
    ]
    .into_iter()
    .map(
        |(uri_template, name, title, description)| ResourceTemplateDescriptor {
            uri_template: uri_template.to_owned(),
            name: name.to_owned(),
            title: title.to_owned(),
            description: description.to_owned(),
            mime_type: "application/json".to_owned(),
            read_only: true,
        },
    )
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
    match error {
        JsonError::ResourceLimitExceeded(message) => Error::resource_limit_exceeded(message),
        other => Error::new(
            ErrorCode::InvalidInput,
            format!("Could not build resource view: {other:?}."),
        ),
    }
}

fn invalid_uri(input: &str, message: &str) -> Error {
    Error::new(
        ErrorCode::InvalidInput,
        format!("{message} Input: {input}."),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_capabilities_uri() {
        let uri = ResourceUri::from_str("pptx://capabilities/v1")
            .expect("capabilities resource URI parses");

        assert_eq!(uri, ResourceUri::Capabilities);
        assert_eq!(uri.to_string(), "pptx://capabilities/v1");
    }

    #[test]
    fn lists_capabilities_resource() {
        let resources = ResourceRegistry::default().list_resources(None);

        assert!(resources.iter().any(|resource| {
            resource.uri == "pptx://capabilities/v1"
                && resource.name == "capabilities"
                && resource.read_only
        }));
    }

    #[test]
    fn schema_descriptors_use_independent_schema_versions() {
        let resources = schema_resource_descriptors();
        let expected = [
            ("agent-view-schema", AGENT_VIEW_VERSION),
            ("patch-schema", PATCH_VERSION),
            ("patch-report-schema", PATCH_REPORT_VERSION),
            ("error-schema", ERROR_VERSION),
        ];

        for (name, version) in expected {
            let resource = resources
                .iter()
                .find(|resource| resource.name == name)
                .unwrap_or_else(|| panic!("missing schema resource {name}"));
            assert!(
                resource.uri.ends_with(&format!("/v{version}")),
                "{name} URI does not use its own version: {}",
                resource.uri
            );
            assert_eq!(
                resource.title,
                format!("{} v{version} schema", name.trim_end_matches("-schema"))
            );
        }
    }

    #[test]
    fn schema_resources_check_versions_per_schema() {
        schema_resource("agent-view", &schema_version(AGENT_VIEW_VERSION))
            .expect("agent-view schema reads at its own version");
        schema_resource("patch", &schema_version(PATCH_VERSION))
            .expect("patch schema reads at its own version");
        schema_resource("patch-report", &schema_version(PATCH_REPORT_VERSION))
            .expect("patch-report schema reads at its own version");
        schema_resource("error", &schema_version(ERROR_VERSION))
            .expect("error schema reads at its own version");

        let unavailable = schema_resource("agent-view", "v999")
            .expect_err("wrong agent-view version is rejected");
        assert_eq!(unavailable.code(), ErrorCode::InvalidInput);
    }

    #[test]
    fn lists_session_resource_templates_without_session() {
        let templates = ResourceRegistry::default().list_resource_templates();
        let uri_templates = templates
            .iter()
            .map(|template| template.uri_template.as_str())
            .collect::<Vec<_>>();

        assert!(uri_templates.contains(&"pptx://sessions/{session_id}/summary"));
        assert!(uri_templates.contains(&"pptx://sessions/{session_id}/slides"));
        assert!(uri_templates.contains(&"pptx://sessions/{session_id}/slides/{slide_id}"));
        assert!(uri_templates.contains(&"pptx://sessions/{session_id}/elements/{element_id}"));
        assert!(uri_templates.contains(&"pptx://sessions/{session_id}/media/{media_id}/metadata"));
        assert!(uri_templates.contains(&"pptx://sessions/{session_id}/validation/latest"));
        assert!(templates.iter().all(|template| template.read_only));
    }

    #[tokio::test]
    async fn reads_capabilities_resource_with_raw_xml_flag() {
        let content = ResourceRegistry::with_raw_xml_enabled(true)
            .read_resource(&ResourceUri::Capabilities, &SessionStore::default())
            .await
            .expect("capabilities resource reads");

        assert_eq!(content.uri, "pptx://capabilities/v1");
        assert_eq!(content.content["schema"], "pptx-compose.capabilities.v1");
        assert_eq!(content.content["raw_xml_enabled"], true);
        assert!(
            content.content["supported_operations"]
                .as_array()
                .expect("operations are an array")
                .iter()
                .any(|operation| operation["op"] == "replace_text")
        );
        assert!(
            content.content["exit_codes"]
                .as_array()
                .expect("exit codes are an array")
                .iter()
                .any(|entry| entry["exit"] == 24
                    && entry["error_codes"]
                        .as_array()
                        .is_some_and(|codes| codes.iter().any(|code| code == "unsupported_edit")))
        );
    }
}
