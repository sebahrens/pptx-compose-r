#![deny(warnings)]

use rmcp::{
    ErrorData, Json, ServerHandler, ServiceExt,
    handler::server::router::tool::ToolRouter,
    model::{
        AnnotateAble, GetPromptRequestParams, GetPromptResult, ListPromptsResult,
        ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams, Prompt,
        PromptMessage, PromptMessageRole, RawResource, RawResourceTemplate,
        ReadResourceRequestParams, ReadResourceResult, Resource, ResourceContents, ServerInfo,
    },
    service::{RequestContext, RoleServer},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use std::str::FromStr;

pub mod outputs;
pub mod permissions;
pub mod prompts;
pub mod resources;
pub mod sessions;
pub mod tools;

use pptx_compose::{
    AgentViewOptions,
    core::error::{Error as CoreError, ErrorCode as CoreErrorCode},
    edit::patch::Patch,
    json::agent_view::{
        FindTextResult, FindTextScope,
        pagination::MAX_PAGE_LIMIT,
        views::{FindTextRequest, ViewMode},
    },
};
use prompts::PromptRegistry;
use resources::{ResourceRegistry, ResourceUri};
use sessions::SessionStore;

const MAX_INLINE_MEDIA_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_MAX_INLINE_EXPORT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerConfig {
    pub max_inline_export_bytes: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            max_inline_export_bytes: DEFAULT_MAX_INLINE_EXPORT_BYTES,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PptxServer {
    sessions: SessionStore,
    config: ServerConfig,
    permission_policy: permissions::PermissionPolicy,
    resource_registry: ResourceRegistry,
    prompt_registry: PromptRegistry,
    tool_router: ToolRouter<Self>,
}

impl Default for PptxServer {
    fn default() -> Self {
        Self::new()
    }
}

impl PptxServer {
    pub fn new() -> Self {
        Self::with_config(ServerConfig::default())
    }

    pub fn with_config(config: ServerConfig) -> Self {
        Self::with_session_store_and_permissions(
            SessionStore::default(),
            config,
            permissions::PermissionPolicy::default(),
        )
    }

    pub fn with_session_store(sessions: SessionStore, config: ServerConfig) -> Self {
        Self::with_session_store_and_permissions(
            sessions,
            config,
            permissions::PermissionPolicy::default(),
        )
    }

    pub fn with_permissions(permission_policy: permissions::PermissionPolicy) -> Self {
        Self::with_session_store_and_permissions(
            SessionStore::default(),
            ServerConfig::default(),
            permission_policy,
        )
    }

    pub fn with_session_store_and_permissions(
        sessions: SessionStore,
        config: ServerConfig,
        permission_policy: permissions::PermissionPolicy,
    ) -> Self {
        Self {
            sessions,
            config,
            permission_policy,
            resource_registry: ResourceRegistry::new(),
            prompt_registry: PromptRegistry::new(),
            tool_router: Self::build_tool_router(),
        }
    }

    pub fn sessions(&self) -> &SessionStore {
        &self.sessions
    }

    pub fn config(&self) -> ServerConfig {
        self.config
    }

    pub fn tool_routes(&self) -> &ToolRouter<Self> {
        &self.tool_router
    }

    pub fn permission_policy(&self) -> &permissions::PermissionPolicy {
        &self.permission_policy
    }

    pub fn resource_registry(&self) -> &ResourceRegistry {
        &self.resource_registry
    }

    pub fn prompt_registry(&self) -> &PromptRegistry {
        &self.prompt_registry
    }
}

fn mcp_error(error: CoreError) -> ErrorData {
    let data = serde_json::to_value(outputs::error_envelope(&error)).ok();
    match error.code() {
        CoreErrorCode::ResourceLimitExceeded
        | CoreErrorCode::UnsupportedPackage
        | CoreErrorCode::ParseError
        | CoreErrorCode::MalformedXml
        | CoreErrorCode::ValidationFailed
        | CoreErrorCode::WriteFailed => {
            ErrorData::new(rmcp::model::ErrorCode(-32000), error.to_string(), data)
        }
        CoreErrorCode::InternalError => ErrorData::internal_error(error.to_string(), data),
        CoreErrorCode::InvalidInput
        | CoreErrorCode::UnsafePath
        | CoreErrorCode::UnsupportedEdit
        | CoreErrorCode::UnsupportedMediaType
        | CoreErrorCode::InvalidBounds
        | CoreErrorCode::StalePatch
        | CoreErrorCode::SelectorNotFound
        | CoreErrorCode::SelectorAmbiguous
        | CoreErrorCode::SelectorGuardFailed
        | CoreErrorCode::MissingMediaRef
        | CoreErrorCode::MediaChecksumMismatch
        | CoreErrorCode::PermissionDenied => ErrorData::invalid_params(error.to_string(), data),
    }
}

fn mcp_resource(descriptor: resources::ResourceDescriptor) -> Resource {
    RawResource::new(descriptor.uri, descriptor.name)
        .with_title(descriptor.title)
        .with_description(descriptor.description)
        .with_mime_type(descriptor.mime_type)
        .no_annotation()
}

fn mcp_resource_template(
    descriptor: resources::ResourceTemplateDescriptor,
) -> rmcp::model::ResourceTemplate {
    RawResourceTemplate::new(descriptor.uri_template, descriptor.name)
        .with_title(descriptor.title)
        .with_description(descriptor.description)
        .with_mime_type(descriptor.mime_type)
        .no_annotation()
}

fn mcp_prompt(descriptor: prompts::PromptDescriptor) -> Prompt {
    Prompt::new(descriptor.name, Some(descriptor.description), None).with_title(descriptor.title)
}

fn mcp_prompt_message(message: prompts::PromptMessage) -> PromptMessage {
    match message.role {
        prompts::PromptRole::User => PromptMessage::new_text(PromptMessageRole::User, message.text),
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenInput {
    #[schemars(
        description = "Readable filesystem path to a .pptx OPC package inside the configured workspace.",
        length(min = 1, max = 4096),
        extend("examples" = ["deck.pptx", "/workspace/input.pptx"])
    )]
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportMediaInput {
    #[schemars(
        description = "Session id returned by pptx_open.",
        length(min = 1, max = 160)
    )]
    pub session_id: String,
    #[schemars(
        description = "Revision guard that must equal the current session revision returned by pptx_open or the last pptx_apply_patch result. Stale values return stale_patch."
    )]
    pub expected_revision: u64,
    #[schemars(
        description = "Readable media file path. Mutually exclusive with inline.",
        length(min = 1, max = 4096),
        extend("examples" = ["assets/logo.png", "/workspace/media/photo.jpg"])
    )]
    pub media_path: Option<String>,
    #[schemars(
        description = "Inline size-limited media bytes. Mutually exclusive with media_path."
    )]
    pub inline: Option<InlineMediaInput>,
    #[schemars(
        description = "Declared image MIME type. V1 accepts image/png, image/jpeg, and image/gif.",
        regex(pattern = "^image/(png|jpeg|gif)$"),
        extend("examples" = ["image/png"])
    )]
    pub content_type: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InlineMediaInput {
    #[serde(default = "default_inline_media_encoding")]
    #[schemars(
        description = "Inline media encoding.",
        regex(pattern = "^base64$"),
        extend("default" = "base64", "examples" = ["base64"])
    )]
    pub encoding: String,
    #[schemars(
        description = "Base64-encoded media bytes. Decoded bytes are capped by the server media limit.",
        length(min = 1, max = 22369624)
    )]
    pub data: String,
}

fn default_inline_media_encoding() -> String {
    "base64".to_owned()
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyPatchInput {
    #[schemars(
        description = "Session id returned by pptx_open.",
        length(min = 1, max = 160)
    )]
    pub session_id: String,
    #[serde(default)]
    #[schemars(
        description = "Optional caller-supplied id echoed in the result envelope.",
        length(min = 1, max = 256)
    )]
    pub client_request_id: Option<String>,
    #[schemars(
        description = "Bounded V1 patch whose document_id and base_revision must match the session. The revision guard lives in patch.base_revision; pptx_apply_patch has no separate expected_revision field."
    )]
    pub patch: Patch,
    #[serde(default)]
    #[schemars(description = "When true, validate without mutating the session.", extend("default" = false))]
    pub dry_run: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidatePatchInput {
    #[schemars(
        description = "Session id returned by pptx_open.",
        length(min = 1, max = 160)
    )]
    pub session_id: String,
    #[schemars(description = "Bounded V1 patch to dry-run against the current session.")]
    pub patch: Patch,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidateInput {
    #[schemars(
        description = "Session id returned by pptx_open.",
        length(min = 1, max = 160)
    )]
    pub session_id: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloseInput {
    #[schemars(
        description = "Session id returned by pptx_open.",
        length(min = 1, max = 160)
    )]
    pub session_id: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportInput {
    #[schemars(
        description = "Session id returned by pptx_open.",
        length(min = 1, max = 160)
    )]
    pub session_id: String,
    #[serde(default)]
    #[schemars(
        description = "Optional caller-supplied id echoed in the result envelope.",
        length(min = 1, max = 256)
    )]
    pub client_request_id: Option<String>,
    #[schemars(
        description = "Revision guard that must equal the current session revision returned by pptx_open or the last pptx_apply_patch result. Stale values return stale_patch."
    )]
    pub expected_revision: u64,
    #[schemars(
        description = "Writable output .pptx path. Required unless inline is true.",
        length(min = 1, max = 4096),
        extend("examples" = ["out/edited.pptx", "/workspace/output.pptx"])
    )]
    pub output_path: Option<String>,
    #[serde(default)]
    #[schemars(description = "Return base64 PPTX bytes in JSON instead of writing a file.", extend("default" = false))]
    pub inline: bool,
    #[serde(default)]
    #[schemars(description = "Permit replacing an existing output_path.", extend("default" = false))]
    pub overwrite: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SummaryInput {
    #[schemars(
        description = "Session id returned by pptx_open.",
        length(min = 1, max = 160)
    )]
    pub session_id: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListSlidesInput {
    #[schemars(
        description = "Session id returned by pptx_open.",
        length(min = 1, max = 160)
    )]
    pub session_id: String,
    #[schemars(
        description = "Opaque pagination cursor returned by a previous response.",
        length(min = 1, max = 256)
    )]
    pub cursor: Option<String>,
    #[schemars(
        description = "Maximum number of slide summaries to return.",
        range(min = 1, max = MAX_PAGE_LIMIT)
    )]
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetSlideInput {
    #[schemars(
        description = "Session id returned by pptx_open.",
        length(min = 1, max = 160)
    )]
    pub session_id: String,
    #[schemars(
        description = "Agent slide id such as slide-1.",
        regex(pattern = "^slide-[1-9][0-9]*$")
    )]
    pub slide_id: String,
    #[schemars(description = "Optional revision guard for read-after-write consistency.")]
    pub expected_revision: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListElementsInput {
    #[schemars(
        description = "Session id returned by pptx_open.",
        length(min = 1, max = 160)
    )]
    pub session_id: String,
    #[schemars(
        description = "Agent slide id such as slide-1.",
        regex(pattern = "^slide-[1-9][0-9]*$")
    )]
    pub slide_id: String,
    #[schemars(
        description = "Opaque pagination cursor returned by a previous response.",
        length(min = 1, max = 256)
    )]
    pub cursor: Option<String>,
    #[schemars(
        description = "Maximum number of elements to return.",
        range(min = 1, max = MAX_PAGE_LIMIT)
    )]
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetElementInput {
    #[schemars(
        description = "Session id returned by pptx_open.",
        length(min = 1, max = 160)
    )]
    pub session_id: String,
    #[schemars(
        description = "Agent element id such as slide-1:shape-3.",
        length(min = 1, max = 256)
    )]
    pub element_id: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindTextInput {
    #[schemars(
        description = "Session id returned by pptx_open.",
        length(min = 1, max = 160)
    )]
    pub session_id: String,
    #[schemars(description = "Text query to search for.", length(min = 1, max = 2048))]
    pub query: String,
    #[serde(default)]
    #[schemars(
        description = "Search scope. Omit for deck-wide search ({}) or pass a slide scope such as {\"scope\":{\"type\":\"slide\",\"slide_id\":\"slide-1\"}}."
    )]
    pub scope: FindTextScope,
    #[schemars(
        description = "Opaque pagination cursor returned by a previous response.",
        length(min = 1, max = 256)
    )]
    pub cursor: Option<String>,
    #[schemars(
        description = "Maximum number of matches to return.",
        range(min = 1, max = MAX_PAGE_LIMIT)
    )]
    pub limit: Option<u32>,
}

fn decode_inline_media(inline: &InlineMediaInput) -> pptx_compose::core::error::Result<Vec<u8>> {
    if inline.encoding != "base64" {
        return Err(pptx_compose::core::error::Error::new(
            pptx_compose::core::error::ErrorCode::InvalidInput,
            "Inline media encoding must be `base64`.",
        ));
    }
    check_inline_encoded_size(&inline.data)?;
    let bytes = decode_base64(&inline.data)?;
    if bytes.len() > MAX_INLINE_MEDIA_BYTES {
        return Err(inline_size_error());
    }
    Ok(bytes)
}

fn check_inline_encoded_size(encoded: &str) -> pptx_compose::core::error::Result<()> {
    let max_encoded_len = MAX_INLINE_MEDIA_BYTES.div_ceil(3).saturating_mul(4);
    if encoded.len() > max_encoded_len {
        return Err(inline_size_error());
    }
    Ok(())
}

fn inline_size_error() -> pptx_compose::core::error::Error {
    pptx_compose::core::error::Error::resource_limit_exceeded(format!(
        "Inline media exceeds max_inline_media_bytes {MAX_INLINE_MEDIA_BYTES}."
    ))
}

fn decode_base64(encoded: &str) -> pptx_compose::core::error::Result<Vec<u8>> {
    let input = encoded.as_bytes();
    if !input.len().is_multiple_of(4) {
        return Err(invalid_base64_error(
            "Inline media contains incomplete base64 data.",
        ));
    }

    let mut bytes = Vec::with_capacity(input.len() / 4 * 3);
    for (index, chunk) in input.chunks_exact(4).enumerate() {
        let is_last = (index + 1) * 4 == input.len();
        push_base64_chunk(chunk, is_last, &mut bytes)?;
    }
    Ok(bytes)
}

fn push_base64_chunk(
    chunk: &[u8],
    is_last: bool,
    bytes: &mut Vec<u8>,
) -> pptx_compose::core::error::Result<()> {
    let first = decode_base64_value(chunk[0])?;
    let second = decode_base64_value(chunk[1])?;

    if chunk[2] == b'=' {
        if chunk[3] != b'=' || !is_last {
            return Err(invalid_base64_error(
                "Inline media contains invalid base64 padding.",
            ));
        }
        bytes.push((first << 2) | (second >> 4));
        return Ok(());
    }

    let third = decode_base64_value(chunk[2])?;
    bytes.push((first << 2) | (second >> 4));
    bytes.push(((second & 0x0f) << 4) | (third >> 2));

    if chunk[3] == b'=' {
        if !is_last {
            return Err(invalid_base64_error(
                "Inline media contains invalid base64 padding.",
            ));
        }
        return Ok(());
    }

    let fourth = decode_base64_value(chunk[3])?;
    bytes.push(((third & 0x03) << 6) | fourth);
    Ok(())
}

fn decode_base64_value(byte: u8) -> pptx_compose::core::error::Result<u8> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(invalid_base64_error(
            "Inline media contains invalid base64 characters.",
        )),
    }
}

fn invalid_base64_error(message: &'static str) -> pptx_compose::core::error::Error {
    pptx_compose::core::error::Error::new(
        pptx_compose::core::error::ErrorCode::InvalidInput,
        message,
    )
}

fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);

        output.push(char::from(ALPHABET[usize::from(first >> 2)]));
        output.push(char::from(
            ALPHABET[usize::from(((first & 0x03) << 4) | (second >> 4))],
        ));
        if chunk.len() > 1 {
            output.push(char::from(
                ALPHABET[usize::from(((second & 0x0f) << 2) | (third >> 6))],
            ));
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(char::from(ALPHABET[usize::from(third & 0x3f)]));
        } else {
            output.push('=');
        }
    }
    output
}

#[tool_router(router = build_tool_router)]
impl PptxServer {
    /// Open a deck into a session.
    #[tool(
        name = "pptx_open",
        annotations(
            title = "Open PPTX",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pptx_open(
        &self,
        input: rmcp::handler::server::wrapper::Parameters<OpenInput>,
    ) -> Result<Json<outputs::OpenOutput>, rmcp::model::CallToolResult> {
        let path = self
            .permission_policy
            .check_read(&input.0.path)
            .map_err(|error| outputs::map_error(error.into_core_error()))?;

        let opened = self.sessions.open_path(path).map_err(outputs::map_error)?;
        Ok(Json(outputs::OpenOutput::opened(opened)))
    }

    /// Return deck summary and capabilities.
    #[tool(
        name = "pptx_get_document_summary",
        annotations(
            title = "Get Document Summary",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub async fn pptx_get_document_summary(
        &self,
        input: rmcp::handler::server::wrapper::Parameters<SummaryInput>,
    ) -> Result<Json<outputs::DocumentSummaryOutput>, rmcp::model::CallToolResult> {
        let content = self
            .resource_registry
            .read_resource(
                &ResourceUri::SessionSummary {
                    session_id: input.0.session_id,
                },
                &self.sessions,
            )
            .await
            .map_err(outputs::map_error)?;
        Ok(Json(outputs::DocumentSummaryOutput::success(
            content.content,
        )))
    }

    /// Page through slide summaries.
    #[tool(
        name = "pptx_list_slides",
        annotations(
            title = "List Slides",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub async fn pptx_list_slides(
        &self,
        input: rmcp::handler::server::wrapper::Parameters<ListSlidesInput>,
    ) -> Result<Json<outputs::ListSlidesOutput>, rmcp::model::CallToolResult> {
        let input = input.0;
        let content = self
            .resource_registry
            .read_resource(
                &ResourceUri::SessionSlides {
                    session_id: input.session_id,
                    cursor: input.cursor,
                    limit: input.limit,
                },
                &self.sessions,
            )
            .await
            .map_err(outputs::map_error)?;
        Ok(Json(outputs::ListSlidesOutput::success(content.content)))
    }

    /// Return one slide view.
    #[tool(
        name = "pptx_get_slide",
        annotations(
            title = "Get Slide",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub async fn pptx_get_slide(
        &self,
        input: rmcp::handler::server::wrapper::Parameters<GetSlideInput>,
    ) -> Result<Json<outputs::SlideOutput>, rmcp::model::CallToolResult> {
        let input = input.0;
        if let Some(expected_revision) = input.expected_revision {
            self.sessions
                .check_revision(&input.session_id, expected_revision)
                .map_err(outputs::map_error)?;
        }
        let content = self
            .resource_registry
            .read_resource(
                &ResourceUri::SessionSlide {
                    session_id: input.session_id,
                    slide_id: input.slide_id,
                },
                &self.sessions,
            )
            .await
            .map_err(outputs::map_error)?;
        Ok(Json(outputs::SlideOutput::success(content.content)))
    }

    /// Page through slide elements.
    #[tool(
        name = "pptx_list_elements",
        annotations(
            title = "List Elements",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub async fn pptx_list_elements(
        &self,
        input: rmcp::handler::server::wrapper::Parameters<ListElementsInput>,
    ) -> Result<Json<outputs::ListElementsOutput>, rmcp::model::CallToolResult> {
        let input = input.0;
        let session = self
            .sessions
            .get(&input.session_id)
            .map_err(outputs::map_error)?;
        let result = session
            .package
            .to_agent_json_with_revision(
                AgentViewOptions {
                    mode: ViewMode::SlideDetail,
                    include_elements: false,
                    slide_id: Some(input.slide_id),
                    slide_ids: Vec::new(),
                    element_id: None,
                    cursor: input.cursor,
                    limit: input.limit,
                },
                session.revision,
            )
            .map_err(outputs::map_error)?;
        Ok(Json(outputs::ListElementsOutput::success(result)))
    }

    /// Return one element detail.
    #[tool(
        name = "pptx_get_element",
        annotations(
            title = "Get Element",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub async fn pptx_get_element(
        &self,
        input: rmcp::handler::server::wrapper::Parameters<GetElementInput>,
    ) -> Result<Json<outputs::ElementOutput>, rmcp::model::CallToolResult> {
        let input = input.0;
        let content = self
            .resource_registry
            .read_resource(
                &ResourceUri::SessionElement {
                    session_id: input.session_id,
                    element_id: input.element_id,
                },
                &self.sessions,
            )
            .await
            .map_err(outputs::map_error)?;
        Ok(Json(outputs::ElementOutput::success(content.content)))
    }

    /// Search text in scoped slides.
    #[tool(
        name = "pptx_find_text",
        annotations(
            title = "Find Text",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub async fn pptx_find_text(
        &self,
        input: rmcp::handler::server::wrapper::Parameters<FindTextInput>,
    ) -> Result<Json<outputs::FindTextOutput>, rmcp::model::CallToolResult> {
        let session = self
            .sessions
            .get(&input.0.session_id)
            .map_err(outputs::map_error)?;
        let result: FindTextResult = session
            .package
            .find_text_with_revision(
                FindTextRequest {
                    query: input.0.query,
                    scope: input.0.scope,
                    cursor: input.0.cursor,
                    limit: input.0.limit,
                },
                session.revision,
            )
            .map_err(outputs::map_error)?;
        Ok(rmcp::Json(outputs::FindTextOutput::found(result)))
    }

    /// Stage media bytes or a path as a media_ref.
    #[tool(
        name = "pptx_import_media",
        annotations(
            title = "Import Media",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pptx_import_media(
        &self,
        input: rmcp::handler::server::wrapper::Parameters<ImportMediaInput>,
    ) -> Result<Json<outputs::ImportMediaOutput>, rmcp::model::CallToolResult> {
        let input = input.0;
        match (input.media_path, input.inline) {
            (Some(_), Some(_)) => Err(outputs::map_error(pptx_compose::core::error::Error::new(
                pptx_compose::core::error::ErrorCode::InvalidInput,
                "pptx_import_media accepts either media_path or inline, not both.",
            ))),
            (Some(media_path), None) => {
                self.sessions
                    .check_revision(&input.session_id, input.expected_revision)
                    .map_err(outputs::map_error)?;
                let media_path = self
                    .permission_policy
                    .check_read(&media_path)
                    .map_err(|error| outputs::map_error(error.into_core_error()))?;
                let handle = self
                    .sessions
                    .import_media_path(
                        &input.session_id,
                        input.expected_revision,
                        media_path,
                        &input.content_type,
                    )
                    .map_err(outputs::map_error)?;
                Ok(Json(outputs::ImportMediaOutput::imported(handle)))
            }
            (None, Some(inline)) => {
                self.sessions
                    .check_revision(&input.session_id, input.expected_revision)
                    .map_err(outputs::map_error)?;
                let bytes = decode_inline_media(&inline).map_err(outputs::map_error)?;
                let handle = self
                    .sessions
                    .import_media_bytes(
                        &input.session_id,
                        input.expected_revision,
                        bytes,
                        &input.content_type,
                    )
                    .map_err(outputs::map_error)?;
                Ok(Json(outputs::ImportMediaOutput::imported(handle)))
            }
            (None, None) => Err(outputs::map_error(pptx_compose::core::error::Error::new(
                pptx_compose::core::error::ErrorCode::InvalidInput,
                "pptx_import_media requires media_path or inline.",
            ))),
        }
    }

    /// Dry-run patch validation.
    #[tool(
        name = "pptx_validate_patch",
        annotations(
            title = "Validate Patch",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub async fn pptx_validate_patch(
        &self,
        input: rmcp::handler::server::wrapper::Parameters<ValidatePatchInput>,
    ) -> Result<Json<outputs::ValidatePatchOutput>, rmcp::model::CallToolResult> {
        self.sessions
            .validate_patch(&input.0.session_id, input.0.patch)
            .map(|report| {
                Json(outputs::ValidatePatchOutput::success(serde_json::json!(
                    report
                )))
            })
            .map_err(outputs::map_error)
    }

    /// Apply an atomic patch to the session.
    #[tool(
        name = "pptx_apply_patch",
        annotations(
            title = "Apply Patch",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pptx_apply_patch(
        &self,
        input: rmcp::handler::server::wrapper::Parameters<ApplyPatchInput>,
    ) -> Result<Json<outputs::ApplyPatchOutput>, rmcp::model::CallToolResult> {
        let input = input.0;
        let client_request_id = input
            .client_request_id
            .clone()
            .unwrap_or_else(|| input.patch.client_request_id.clone());
        let transaction_id = outputs::transaction_id().map_err(outputs::map_error)?;
        let result = self
            .sessions
            .apply_patch(&input.session_id, input.patch, input.dry_run)
            .map_err(outputs::map_error)?;
        Ok(Json(outputs::ApplyPatchOutput::applied(
            &input.session_id,
            result.revision,
            input.dry_run,
            &client_request_id,
            &transaction_id,
            result.report,
        )))
    }

    /// Validate the current session.
    #[tool(
        name = "pptx_validate",
        annotations(
            title = "Validate PPTX",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub async fn pptx_validate(
        &self,
        input: rmcp::handler::server::wrapper::Parameters<ValidateInput>,
    ) -> Result<Json<outputs::ValidateOutput>, rmcp::model::CallToolResult> {
        let latest = self
            .sessions
            .validate_session(&input.0.session_id)
            .map_err(outputs::map_error)?;
        Ok(Json(outputs::ValidateOutput::success(serde_json::json!(
            latest.report
        ))))
    }

    /// Write or return PPTX output.
    #[tool(
        name = "pptx_export",
        annotations(
            title = "Export PPTX",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pptx_export(
        &self,
        input: rmcp::handler::server::wrapper::Parameters<ExportInput>,
    ) -> Result<Json<outputs::ExportOutput>, rmcp::model::CallToolResult> {
        let input = input.0;
        let transaction_id = outputs::transaction_id().map_err(outputs::map_error)?;
        if input.output_path.is_some() && input.inline {
            return Err(outputs::map_error(
                pptx_compose::core::error::Error::new(
                    pptx_compose::core::error::ErrorCode::InvalidInput,
                    "pptx_export accepts either output_path or inline=true, not both.",
                )
                .with_suggestion(
                    "Set output_path for file export, or omit output_path and set inline=true for a size-limited JSON export.",
                ),
            ));
        }
        if input.output_path.is_none() && !input.inline {
            return Err(outputs::map_error(
                pptx_compose::core::error::Error::new(
                    pptx_compose::core::error::ErrorCode::InvalidInput,
                    "pptx_export requires output_path unless inline is explicitly true.",
                )
                .with_suggestion("Set output_path for file export, or set inline=true for a size-limited JSON export."),
            ));
        }
        self.sessions
            .check_revision(&input.session_id, input.expected_revision)
            .map_err(outputs::map_error)?;
        let changed_parts = self
            .sessions
            .changed_parts(&input.session_id)
            .map_err(outputs::map_error)?;
        if let Some(output_path) = input.output_path {
            let allow_overwrite = input.overwrite && self.permission_policy.allow_overwrite;
            let output_path = self
                .permission_policy
                .check_write_with_overwrite(&output_path, allow_overwrite)
                .map_err(|error| outputs::map_error(error.into_core_error()))?;
            let temp_path = pptx_compose::temp_output_path(
                &output_path,
                Some(&self.permission_policy.temp_dir),
            );
            let temp_path = self
                .permission_policy
                .check_write_with_overwrite(&temp_path, true)
                .map_err(|error| outputs::map_error(error.into_core_error()))?;
            let metadata = self
                .sessions
                .export_path(
                    &input.session_id,
                    input.expected_revision,
                    &output_path,
                    allow_overwrite,
                    temp_path,
                )
                .map_err(outputs::map_error)?;
            return Ok(Json(outputs::ExportOutput::exported(
                input.session_id,
                input.client_request_id,
                transaction_id,
                changed_parts,
                serde_json::json!({
                    "output_path": output_path,
                    "byte_length": metadata.byte_length,
                    "sha256": metadata.sha256,
                    "inline": null
                }),
            )));
        }

        let bytes = self
            .sessions
            .export_bytes(&input.session_id, input.expected_revision)
            .map_err(outputs::map_error)?;
        if bytes.len() > self.config.max_inline_export_bytes {
            return Err(outputs::map_error(
                pptx_compose::core::error::Error::resource_limit_exceeded(format!(
                    "Inline export is {} bytes, exceeding max_inline_export_bytes {}.",
                    bytes.len(),
                    self.config.max_inline_export_bytes
                ))
                .with_suggestion("Export to output_path instead of requesting inline PPTX bytes."),
            ));
        }
        Ok(Json(outputs::ExportOutput::exported(
            input.session_id,
            input.client_request_id,
            transaction_id,
            changed_parts,
            serde_json::json!({
                "output_path": null,
                "byte_length": bytes.len(),
                "sha256": sessions::sha256_hex(&bytes),
                "inline": {
                    "encoding": "base64",
                    "data": encode_base64(&bytes)
                }
            }),
        )))
    }

    /// Release session resources.
    #[tool(
        name = "pptx_close",
        annotations(
            title = "Close PPTX Session",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub async fn pptx_close(
        &self,
        input: rmcp::handler::server::wrapper::Parameters<CloseInput>,
    ) -> Result<Json<outputs::CloseOutput>, rmcp::model::CallToolResult> {
        let closed = self
            .sessions
            .close(&input.0.session_id)
            .map_err(outputs::map_error)?;
        Ok(Json(outputs::CloseOutput::closed(
            &input.0.session_id,
            closed,
        )))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for PptxServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
        )
        .with_server_info(rmcp::model::Implementation::new(
            "pptx-compose-mcp",
            env!("CARGO_PKG_VERSION"),
        ))
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(
            self.resource_registry
                .list_resources(None)
                .into_iter()
                .map(mcp_resource)
                .collect(),
        ))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        let uri = ResourceUri::from_str(&request.uri).map_err(mcp_error)?;
        let content = self
            .resource_registry
            .read_resource(&uri, &self.sessions)
            .await
            .map_err(mcp_error)?;
        let text = serde_json::to_string(&content.content).map_err(|source| {
            ErrorData::internal_error(
                format!("Could not serialize resource JSON: {source}."),
                None,
            )
        })?;

        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(text, content.uri).with_mime_type(content.mime_type),
        ]))
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        Ok(ListPromptsResult::with_all_items(
            self.prompt_registry
                .list_prompts()
                .into_iter()
                .map(mcp_prompt)
                .collect(),
        ))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, ErrorData> {
        let messages = self
            .prompt_registry
            .build_messages(&request.name)
            .map_err(mcp_error)?
            .into_iter()
            .map(mcp_prompt_message)
            .collect();
        Ok(GetPromptResult::new(messages))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        Ok(ListResourceTemplatesResult::with_all_items(
            self.resource_registry
                .list_resource_templates()
                .into_iter()
                .map(mcp_resource_template)
                .collect(),
        ))
    }
}

pub async fn run() -> Result<(), rmcp::service::ServerInitializeError> {
    run_server(PptxServer::default()).await
}

pub async fn run_server(server: PptxServer) -> Result<(), rmcp::service::ServerInitializeError> {
    let service = server.serve(rmcp::transport::io::stdio()).await?;
    service
        .waiting()
        .await
        .map(|_| ())
        .map_err(|error| rmcp::service::ServerInitializeError::ConnectionClosed(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use pptx_compose::core::error::ErrorCode;

    use super::*;

    const ONE_BY_ONE_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=";

    #[test]
    fn mcp_error_attaches_error_envelope_to_invalid_params() {
        let error = CoreError::new(ErrorCode::InvalidInput, "Bad resource URI.");
        let mapped = mcp_error(error);
        let data = mapped.data.expect("error envelope is attached");

        assert_eq!(mapped.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert_eq!(data["schema"], "pptx-compose.error.v1");
        assert_eq!(data["status"], "error");
        assert_eq!(data["error"]["code"], "invalid_input");
        assert_eq!(data["error"]["category"], "input");
    }

    #[test]
    fn mcp_error_maps_resource_limit_to_server_error_with_envelope() {
        let error = CoreError::resource_limit_exceeded("Slide page limit exceeded.");
        let mapped = mcp_error(error);
        let data = mapped.data.expect("error envelope is attached");

        assert_eq!(mapped.code, rmcp::model::ErrorCode(-32000));
        assert_eq!(data["schema"], "pptx-compose.error.v1");
        assert_eq!(data["error"]["code"], "resource_limit_exceeded");
        assert_eq!(data["error"]["retryable"], false);
    }

    #[test]
    fn mcp_error_maps_internal_error_to_json_rpc_internal_error() {
        let error = CoreError::new(
            ErrorCode::InternalError,
            "Could not serialize resource JSON.",
        );
        let mapped = mcp_error(error);
        let data = mapped.data.expect("error envelope is attached");

        assert_eq!(mapped.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        assert_eq!(data["schema"], "pptx-compose.error.v1");
        assert_eq!(data["error"]["code"], "internal_error");
        assert_eq!(data["error"]["category"], "internal");
    }

    #[test]
    fn list_elements_input_requires_slide_id() {
        let missing_slide_id = serde_json::json!({
            "session_id": "sess_1",
            "limit": 10
        });

        serde_json::from_value::<ListElementsInput>(missing_slide_id)
            .expect_err("slide_id is required");
    }

    #[tokio::test]
    async fn list_elements_returns_slide_detail_element_page() {
        let server = PptxServer::default();
        let opened = open_fixture_session(&server);

        let listed = server
            .pptx_list_elements(rmcp::handler::server::wrapper::Parameters(
                ListElementsInput {
                    session_id: opened.session_id,
                    slide_id: "slide-1".to_owned(),
                    cursor: None,
                    limit: Some(1),
                },
            ))
            .await
            .expect("list elements succeeds");

        let result = listed.0.0.result;
        assert_eq!(result["view"]["mode"], "slide_detail");
        assert_eq!(result["view"]["limit"], 1);
        assert_eq!(result["slides"][0]["id"], "slide-1");
        assert!(
            result["slides"][0]["elements"]
                .as_array()
                .is_some_and(|elements| elements.len() <= 1)
        );
    }

    #[tokio::test]
    async fn find_text_defaults_to_deck_scope_when_scope_is_omitted() {
        let server = PptxServer::default();
        let deck = custom_slide_path_deck();
        let opened = server
            .sessions()
            .open_package(
                pptx_compose::PresentationDocument::from_bytes(deck.clone())
                    .expect("text deck opens"),
                &deck,
            )
            .expect("session opens");
        let input = serde_json::from_value::<FindTextInput>(serde_json::json!({
            "session_id": opened.session_id,
            "query": "Deck Needle"
        }))
        .expect("find_text scope defaults during deserialization");

        assert_eq!(input.scope, FindTextScope::Deck);

        let found = server
            .pptx_find_text(rmcp::handler::server::wrapper::Parameters(input))
            .await
            .expect("find_text succeeds with omitted scope");
        let result = found.0.0.result;

        assert_eq!(result["scope"]["type"], "deck");
        assert!(
            result["matches"]
                .as_array()
                .is_some_and(|matches| !matches.is_empty()),
            "deck-wide search returns matches"
        );
    }

    #[tokio::test]
    async fn import_media_accepts_inline_base64_without_filesystem_access() {
        let server = PptxServer::default();
        let opened = open_fixture_session(&server);

        let imported = server
            .pptx_import_media(rmcp::handler::server::wrapper::Parameters(
                ImportMediaInput {
                    session_id: opened.session_id.clone(),
                    expected_revision: opened.revision,
                    media_path: None,
                    inline: Some(InlineMediaInput {
                        encoding: "base64".to_owned(),
                        data: ONE_BY_ONE_PNG_BASE64.to_owned(),
                    }),
                    content_type: "image/png".to_owned(),
                },
            ))
            .await
            .expect("inline media imports");

        let result = imported.0.0.result;
        assert_eq!(result["media_ref"], "media_1");
        assert_eq!(result["content_type"], "image/png");
        assert_eq!(result["byte_length"], 68);
        assert_eq!(result["dimensions_px"]["width"], 1);
        assert_eq!(result["dimensions_px"]["height"], 1);

        let session = server
            .sessions()
            .get(&opened.session_id)
            .expect("session remains open");
        assert!(session.media.contains_key("media_1"));
    }

    #[tokio::test]
    async fn import_media_defaults_inline_base64_encoding() {
        let server = PptxServer::default();
        let opened = open_fixture_session(&server);
        let input = serde_json::from_value::<ImportMediaInput>(serde_json::json!({
            "session_id": opened.session_id,
            "expected_revision": opened.revision,
            "inline": {
                "data": ONE_BY_ONE_PNG_BASE64
            },
            "content_type": "image/png"
        }))
        .expect("inline encoding defaults during deserialization");

        assert_eq!(
            input.inline.as_ref().expect("inline media input").encoding,
            "base64"
        );

        let imported = server
            .pptx_import_media(rmcp::handler::server::wrapper::Parameters(input))
            .await
            .expect("inline media imports with default encoding");

        let result = imported.0.0.result;
        assert_eq!(result["media_ref"], "media_1");
        assert_eq!(result["content_type"], "image/png");
        assert_eq!(result["byte_length"], 68);
    }

    #[tokio::test]
    async fn import_media_rejects_stale_revision() {
        let server = PptxServer::default();
        let opened = open_fixture_session(&server);
        server
            .sessions()
            .record_apply(&opened.session_id, opened.revision, false, true)
            .expect("test revision increments");

        let result = server
            .pptx_import_media(rmcp::handler::server::wrapper::Parameters(
                ImportMediaInput {
                    session_id: opened.session_id.clone(),
                    expected_revision: opened.revision,
                    media_path: None,
                    inline: Some(InlineMediaInput {
                        encoding: "base64".to_owned(),
                        data: ONE_BY_ONE_PNG_BASE64.to_owned(),
                    }),
                    content_type: "image/png".to_owned(),
                },
            ))
            .await;

        let Err(error) = result else {
            panic!("stale import_media revision is rejected");
        };
        let envelope = error
            .structured_content
            .expect("stale import_media error has structured content");

        assert_eq!(error.is_error, Some(true));
        assert_eq!(envelope["error"]["code"], ErrorCode::StalePatch.as_str());
        assert_eq!(envelope["error"]["location"]["current_revision"], 2);
        assert!(
            server
                .sessions()
                .get(&opened.session_id)
                .expect("session remains open")
                .media
                .is_empty(),
            "stale import does not stage media"
        );
    }

    #[tokio::test]
    async fn stale_import_media_rejects_before_decoding_bad_inline() {
        let server = PptxServer::default();
        let opened = open_fixture_session(&server);
        server
            .sessions()
            .record_apply(&opened.session_id, opened.revision, false, true)
            .expect("test revision increments");
        let oversized_encoded =
            "A".repeat(MAX_INLINE_MEDIA_BYTES.div_ceil(3).saturating_mul(4) + 4);

        for data in ["!!!!".to_owned(), oversized_encoded] {
            let result = server
                .pptx_import_media(rmcp::handler::server::wrapper::Parameters(
                    ImportMediaInput {
                        session_id: opened.session_id.clone(),
                        expected_revision: opened.revision,
                        media_path: None,
                        inline: Some(InlineMediaInput {
                            encoding: "base64".to_owned(),
                            data,
                        }),
                        content_type: "image/png".to_owned(),
                    },
                ))
                .await;

            let Err(error) = result else {
                panic!("stale bad inline import is rejected");
            };
            let envelope = error
                .structured_content
                .expect("stale inline error has structured content");
            assert_eq!(error.is_error, Some(true));
            assert_eq!(envelope["error"]["code"], ErrorCode::StalePatch.as_str());
        }
    }

    #[tokio::test]
    async fn stale_import_media_rejects_before_checking_missing_path() {
        let server = PptxServer::default();
        let opened = open_fixture_session(&server);
        server
            .sessions()
            .record_apply(&opened.session_id, opened.revision, false, true)
            .expect("test revision increments");
        let missing_path = std::env::temp_dir().join(format!(
            "pptx-compose-mcp-missing-media-{}-{}.png",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));

        let result = server
            .pptx_import_media(rmcp::handler::server::wrapper::Parameters(
                ImportMediaInput {
                    session_id: opened.session_id,
                    expected_revision: opened.revision,
                    media_path: Some(missing_path.to_string_lossy().into_owned()),
                    inline: None,
                    content_type: "image/png".to_owned(),
                },
            ))
            .await;

        let Err(error) = result else {
            panic!("stale missing-path import is rejected");
        };
        let envelope = error
            .structured_content
            .expect("stale path error has structured content");
        assert_eq!(error.is_error, Some(true));
        assert_eq!(envelope["error"]["code"], ErrorCode::StalePatch.as_str());
    }

    #[tokio::test]
    async fn import_media_rejects_inline_content_type_mismatch() {
        let server = PptxServer::default();
        let opened = open_fixture_session(&server);

        let result = server
            .pptx_import_media(rmcp::handler::server::wrapper::Parameters(
                ImportMediaInput {
                    session_id: opened.session_id,
                    expected_revision: opened.revision,
                    media_path: None,
                    inline: Some(InlineMediaInput {
                        encoding: "base64".to_owned(),
                        data: ONE_BY_ONE_PNG_BASE64.to_owned(),
                    }),
                    content_type: "image/jpeg".to_owned(),
                },
            ))
            .await;

        let Err(error) = result else {
            panic!("content-type mismatch is rejected");
        };
        let envelope = error
            .structured_content
            .expect("mismatch error has structured content");

        assert_eq!(error.is_error, Some(true));
        assert_eq!(
            envelope["error"]["code"],
            ErrorCode::UnsupportedMediaType.as_str()
        );
    }

    #[tokio::test]
    async fn export_rejects_stale_revision() {
        let server = PptxServer::default();
        let opened = open_fixture_session(&server);
        server
            .sessions()
            .record_apply(&opened.session_id, opened.revision, false, true)
            .expect("test revision increments");

        let result = server
            .pptx_export(rmcp::handler::server::wrapper::Parameters(ExportInput {
                session_id: opened.session_id,
                client_request_id: None,
                expected_revision: opened.revision,
                output_path: None,
                inline: true,
                overwrite: false,
            }))
            .await;

        let Err(error) = result else {
            panic!("stale export revision is rejected");
        };
        let envelope = error
            .structured_content
            .expect("stale export error has structured content");

        assert_eq!(error.is_error, Some(true));
        assert_eq!(envelope["error"]["code"], ErrorCode::StalePatch.as_str());
        assert_eq!(envelope["error"]["location"]["current_revision"], 2);
    }

    #[tokio::test]
    async fn export_returns_inline_bytes_when_inline_only() {
        let server = PptxServer::default();
        let opened = open_fixture_session(&server);

        let exported = server
            .pptx_export(rmcp::handler::server::wrapper::Parameters(ExportInput {
                session_id: opened.session_id,
                client_request_id: None,
                expected_revision: opened.revision,
                output_path: None,
                inline: true,
                overwrite: false,
            }))
            .await
            .expect("inline export succeeds");

        assert_eq!(exported.0.0.result["output_path"], serde_json::Value::Null);
        assert_eq!(exported.0.0.result["inline"]["encoding"], "base64");
        assert!(
            exported.0.0.result["inline"]["data"]
                .as_str()
                .is_some_and(|data| !data.is_empty())
        );
    }

    #[tokio::test]
    async fn export_writes_output_path_when_path_only() {
        let server = PptxServer::default();
        let opened = open_fixture_session(&server);
        let output_path = std::env::temp_dir().join(format!(
            "pptx-compose-mcp-path-export-{}-{}.pptx",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));

        let exported = server
            .pptx_export(rmcp::handler::server::wrapper::Parameters(ExportInput {
                session_id: opened.session_id,
                client_request_id: None,
                expected_revision: opened.revision,
                output_path: Some(output_path.to_string_lossy().into_owned()),
                inline: false,
                overwrite: false,
            }))
            .await
            .expect("path export succeeds");

        let exported_bytes = fs::read(&output_path).expect("path export writes file");
        let canonical_output_path =
            fs::canonicalize(&output_path).expect("path export canonicalizes");
        fs::remove_file(&output_path).expect("remove path export fixture");
        assert_eq!(
            exported.0.0.result["output_path"],
            canonical_output_path.to_string_lossy().as_ref()
        );
        assert_eq!(exported.0.0.result["byte_length"], exported_bytes.len());
        assert_eq!(
            exported.0.0.result["sha256"],
            sessions::sha256_hex(&exported_bytes)
        );
        assert_eq!(exported.0.0.result["inline"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn export_overwrite_requires_server_policy_and_client_request() {
        let root = std::env::temp_dir().join(format!(
            "pptx-compose-mcp-overwrite-policy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let temp_dir = root.join("tmp");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let output_path = workspace.join("existing.pptx");
        fs::write(&output_path, b"existing output").expect("write existing output");

        let server = PptxServer::with_permissions(permissions::PermissionPolicy::new(
            workspace.clone(),
            temp_dir,
            false,
        ));
        let opened = open_fixture_session(&server);

        let result = server
            .pptx_export(rmcp::handler::server::wrapper::Parameters(ExportInput {
                session_id: opened.session_id,
                client_request_id: None,
                expected_revision: opened.revision,
                output_path: Some(output_path.to_string_lossy().into_owned()),
                inline: false,
                overwrite: true,
            }))
            .await;

        let Err(error) = result else {
            panic!("server policy rejects client overwrite request");
        };
        let envelope = error
            .structured_content
            .expect("overwrite policy error has structured content");
        assert_eq!(error.is_error, Some(true));
        assert_eq!(
            envelope["error"]["code"],
            ErrorCode::PermissionDenied.as_str()
        );
        assert_eq!(
            fs::read(&output_path).expect("existing output remains"),
            b"existing output"
        );

        fs::remove_dir_all(root).expect("remove overwrite policy fixture");
    }

    #[tokio::test]
    async fn export_rejects_output_path_with_inline() {
        let server = PptxServer::default();
        let opened = open_fixture_session(&server);
        let output_path = std::env::temp_dir().join(format!(
            "pptx-compose-mcp-combined-export-{}-{}.pptx",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));

        let result = server
            .pptx_export(rmcp::handler::server::wrapper::Parameters(ExportInput {
                session_id: opened.session_id,
                client_request_id: None,
                expected_revision: opened.revision,
                output_path: Some(output_path.to_string_lossy().into_owned()),
                inline: true,
                overwrite: false,
            }))
            .await;

        let Err(error) = result else {
            panic!("combined path and inline export is rejected");
        };
        let envelope = error
            .structured_content
            .expect("combined export error has structured content");
        assert_eq!(error.is_error, Some(true));
        assert_eq!(envelope["error"]["code"], ErrorCode::InvalidInput.as_str());
        assert!(
            envelope["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("output_path or inline"))
        );
        assert!(!output_path.exists());
    }

    #[tokio::test]
    async fn stale_export_rejects_before_checking_existing_output_path() {
        let server = PptxServer::default();
        let opened = open_fixture_session(&server);
        server
            .sessions()
            .record_apply(&opened.session_id, opened.revision, false, true)
            .expect("test revision increments");
        let output_path = std::env::temp_dir().join(format!(
            "pptx-compose-mcp-existing-output-{}-{}.pptx",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        fs::write(&output_path, b"existing output").expect("existing output fixture");

        let result = server
            .pptx_export(rmcp::handler::server::wrapper::Parameters(ExportInput {
                session_id: opened.session_id,
                client_request_id: None,
                expected_revision: opened.revision,
                output_path: Some(output_path.to_string_lossy().into_owned()),
                inline: false,
                overwrite: false,
            }))
            .await;

        fs::remove_file(&output_path).expect("remove output fixture");
        let Err(error) = result else {
            panic!("stale existing-path export is rejected");
        };
        let envelope = error
            .structured_content
            .expect("stale export path error has structured content");
        assert_eq!(error.is_error, Some(true));
        assert_eq!(envelope["error"]["code"], ErrorCode::StalePatch.as_str());
    }

    #[test]
    fn inline_base64_decoder_rejects_invalid_input() {
        let bad_encoding = decode_inline_media(&InlineMediaInput {
            encoding: "hex".to_owned(),
            data: String::new(),
        })
        .expect_err("non-base64 encoding fails");
        assert_eq!(bad_encoding.code(), ErrorCode::InvalidInput);

        let bad_data = decode_inline_media(&InlineMediaInput {
            encoding: "base64".to_owned(),
            data: "!!!!".to_owned(),
        })
        .expect_err("malformed base64 fails");
        assert_eq!(bad_data.code(), ErrorCode::InvalidInput);
    }

    #[test]
    fn read_and_export_tools_expose_input_schemas() {
        assert_tool_schema_fields("pptx_get_document_summary", &["session_id"], &[]);
        assert_tool_schema_fields("pptx_list_slides", &["session_id"], &["cursor", "limit"]);
        assert_tool_schema_fields(
            "pptx_get_slide",
            &["session_id", "slide_id"],
            &["expected_revision"],
        );
        assert_tool_schema_fields(
            "pptx_list_elements",
            &["session_id", "slide_id"],
            &["cursor", "limit"],
        );
        assert_tool_schema_fields("pptx_get_element", &["session_id", "element_id"], &[]);
        assert_tool_schema_fields(
            "pptx_find_text",
            &["session_id", "query"],
            &["scope", "cursor", "limit"],
        );
        assert_tool_schema_fields(
            "pptx_import_media",
            &["session_id", "expected_revision", "content_type"],
            &["media_path", "inline"],
        );
        assert_tool_schema_fields(
            "pptx_export",
            &["session_id", "expected_revision"],
            &["output_path", "overwrite"],
        );
    }

    #[test]
    fn tools_expose_output_schemas_and_contract_annotations() {
        let tools = crate::tools::exposed_tools(&PptxServer::default());
        let success_output = serde_json::json!({
            "schema": "pptx-compose.result.v1",
            "version": 1,
            "status": "success",
            "result": {},
            "warnings": [],
            "next_cursor": null
        });
        let error_output = serde_json::to_value(outputs::error_envelope(&CoreError::new(
            ErrorCode::InvalidInput,
            "Invalid tool input.",
        )))
        .expect("error envelope serializes");
        for tool_name in crate::tools::DEFAULT_TOOL_NAMES {
            let tool = tools
                .iter()
                .find(|tool| tool.name.as_ref() == *tool_name)
                .unwrap_or_else(|| panic!("tool {tool_name} is exposed"));
            let schema = tool
                .output_schema
                .as_ref()
                .unwrap_or_else(|| panic!("tool {tool_name} exposes output schema"));
            assert_eq!(
                schema.get("type"),
                Some(&serde_json::Value::String("object".to_owned())),
                "tool {tool_name} output schema is object-rooted"
            );
            assert!(
                schema.get("anyOf").is_some_and(serde_json::Value::is_array),
                "tool {tool_name} output schema is a result/error union"
            );
            let validator =
                jsonschema::validator_for(&serde_json::Value::Object(schema.as_ref().clone()))
                    .unwrap_or_else(|error| {
                        panic!("tool {tool_name} output schema compiles: {error}")
                    });
            assert!(
                validator.is_valid(&success_output),
                "tool {tool_name} output schema accepts success envelopes"
            );
            assert!(
                validator.is_valid(&error_output),
                "tool {tool_name} output schema accepts error envelopes"
            );
        }

        let import_media = tools
            .iter()
            .find(|tool| tool.name.as_ref() == "pptx_import_media")
            .expect("import media tool is exposed");
        let annotations = import_media
            .annotations
            .as_ref()
            .expect("import media annotations are exposed");
        assert_eq!(annotations.read_only_hint, Some(false));
        assert_eq!(annotations.destructive_hint, Some(true));
        assert_eq!(annotations.idempotent_hint, Some(false));
    }

    #[test]
    fn tool_input_schemas_describe_limits_and_encodings() {
        let tools = crate::tools::exposed_tools(&PptxServer::default());
        let import_media = tools
            .iter()
            .find(|tool| tool.name.as_ref() == "pptx_import_media")
            .expect("import media tool is exposed");
        let schema = import_media.schema_as_json_value();
        let properties = schema["properties"].as_object().expect("properties");

        assert_eq!(
            properties["content_type"]["pattern"],
            "^image/(png|jpeg|gif)$"
        );
        assert!(
            properties["media_path"]["description"]
                .as_str()
                .is_some_and(|value| value.contains("Readable media file path"))
        );
        assert_eq!(
            properties["inline"]["anyOf"][0]["$ref"],
            "#/$defs/InlineMediaInput"
        );
        assert!(
            schema["$defs"]["InlineMediaInput"]["properties"]["data"]["maxLength"]
                .as_u64()
                .is_some_and(|value| value > 0)
        );
    }

    #[test]
    fn paginated_tool_input_schemas_match_runtime_page_limit() {
        for tool_name in ["pptx_list_slides", "pptx_list_elements", "pptx_find_text"] {
            let maximum = limit_schema_maximum(tool_name);
            assert_eq!(
                maximum,
                Some(u64::from(MAX_PAGE_LIMIT)),
                "tool {tool_name} limit schema maximum must match runtime page limit"
            );
        }
    }

    #[test]
    fn mutating_tool_input_schemas_name_revision_guards() {
        let tools = crate::tools::exposed_tools(&PptxServer::default());

        let import_media = tools
            .iter()
            .find(|tool| tool.name.as_ref() == "pptx_import_media")
            .expect("import media tool is exposed");
        let import_media_schema = import_media.schema_as_json_value();
        assert!(
            import_media_schema["properties"]["expected_revision"]["description"]
                .as_str()
                .is_some_and(|value| value.contains("last pptx_apply_patch result")),
            "pptx_import_media expected_revision explains the current-session guard"
        );

        let apply_patch = tools
            .iter()
            .find(|tool| tool.name.as_ref() == "pptx_apply_patch")
            .expect("apply patch tool is exposed");
        let apply_patch_schema = apply_patch.schema_as_json_value();
        let patch_description = apply_patch_schema["properties"]["patch"]["description"]
            .as_str()
            .expect("patch description is present");
        assert!(patch_description.contains("patch.base_revision"));
        assert!(patch_description.contains("no separate expected_revision"));

        let export = tools
            .iter()
            .find(|tool| tool.name.as_ref() == "pptx_export")
            .expect("export tool is exposed");
        let export_schema = export.schema_as_json_value();
        assert!(
            export_schema["properties"]["expected_revision"]["description"]
                .as_str()
                .is_some_and(|value| value.contains("last pptx_apply_patch result")),
            "pptx_export expected_revision explains the current-session guard"
        );
    }

    #[tokio::test]
    async fn validate_records_latest_validation_resource() {
        let server = PptxServer::default();
        let opened = open_fixture_session(&server);
        let before = server
            .resource_registry()
            .read_resource(
                &ResourceUri::SessionLatestValidation {
                    session_id: opened.session_id.clone(),
                },
                server.sessions(),
            )
            .await
            .expect("latest validation resource reads before validation");
        assert_eq!(before.content["status"], "not_yet_validated");
        assert_eq!(before.content["revision"], opened.revision);

        server
            .pptx_validate(rmcp::handler::server::wrapper::Parameters(ValidateInput {
                session_id: opened.session_id.clone(),
            }))
            .await
            .expect("validation succeeds");

        let after = server
            .resource_registry()
            .read_resource(
                &ResourceUri::SessionLatestValidation {
                    session_id: opened.session_id.clone(),
                },
                server.sessions(),
            )
            .await
            .expect("latest validation resource reads after validation");
        assert_eq!(after.content["status"], "validated");
        assert_eq!(after.content["revision"], opened.revision);
        assert_eq!(after.content["source"], "tool");
        assert_eq!(after.content["report"]["revision"], opened.revision);
    }

    #[test]
    fn open_metadata_counts_slides_from_relationship_graph() {
        let server = PptxServer::default();
        let deck = custom_slide_path_deck();
        let opened = server
            .sessions()
            .open_package(
                pptx_compose::PresentationDocument::from_bytes(deck.clone())
                    .expect("custom-path fixture opens"),
                &deck,
            )
            .expect("session opens");

        assert_eq!(opened.slide_count, 1);
    }

    fn assert_tool_schema_fields(tool_name: &str, required: &[&str], optional: &[&str]) {
        let tools = crate::tools::exposed_tools(&PptxServer::default());
        let tool = tools
            .iter()
            .find(|tool| tool.name.as_ref() == tool_name)
            .unwrap_or_else(|| panic!("tool {tool_name} is exposed"));
        let schema = tool.schema_as_json_value();
        let properties = schema["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("tool {tool_name} has object properties"));
        let required_fields = schema["required"]
            .as_array()
            .unwrap_or_else(|| panic!("tool {tool_name} has required fields"));

        for field in required.iter().chain(optional.iter()) {
            assert!(
                properties.contains_key(*field),
                "tool {tool_name} schema omits property {field}"
            );
        }
        for field in required {
            assert!(
                required_fields.iter().any(|value| value == *field),
                "tool {tool_name} schema does not require {field}"
            );
        }
        for field in optional {
            assert!(
                required_fields.iter().all(|value| value != *field),
                "tool {tool_name} schema unexpectedly requires {field}"
            );
        }
    }

    fn limit_schema_maximum(tool_name: &str) -> Option<u64> {
        let tools = crate::tools::exposed_tools(&PptxServer::default());
        let tool = tools
            .iter()
            .find(|tool| tool.name.as_ref() == tool_name)
            .unwrap_or_else(|| panic!("tool {tool_name} is exposed"));
        let schema = tool.schema_as_json_value();
        schema["properties"]["limit"]["maximum"].as_u64()
    }

    fn open_fixture_session(server: &PptxServer) -> sessions::OpenSession {
        let fixture = include_bytes!("../../../fixtures/minimal.pptx").to_vec();
        server
            .sessions()
            .open_package(
                pptx_compose::PresentationDocument::from_bytes(fixture.clone())
                    .expect("fixture opens"),
                &fixture,
            )
            .expect("session opens")
    }

    fn custom_slide_path_deck() -> Vec<u8> {
        use std::io::{Cursor, Write};
        use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

        let content_types = custom_content_types();
        let root_rels = custom_root_rels();
        let presentation = custom_presentation();
        let presentation_rels = custom_presentation_rels();
        let slide = custom_slide();
        let entries = [
            ("[Content_Types].xml", content_types.as_bytes()),
            ("_rels/.rels", root_rels.as_bytes()),
            ("custom/presentation.xml", presentation.as_bytes()),
            (
                "custom/_rels/presentation.xml.rels",
                presentation_rels.as_bytes(),
            ),
            ("custom/slides/title.xml", slide.as_bytes()),
        ];
        let mut bytes = Vec::new();
        {
            let mut writer = ZipWriter::new(Cursor::new(&mut bytes));
            let options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            for (name, data) in entries {
                writer.start_file(name, options).expect("start ZIP entry");
                writer.write_all(data).expect("write ZIP entry");
            }
            writer.finish().expect("finish ZIP");
        }
        bytes
    }

    fn custom_content_types() -> String {
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/custom/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/custom/slides/title.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
</Types>"#
            .to_owned()
    }

    fn custom_root_rels() -> String {
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="custom/presentation.xml"/>
</Relationships>"#
            .to_owned()
    }

    fn custom_presentation() -> String {
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:sldIdLst><p:sldId id="256" r:id="rCustomSlide"/></p:sldIdLst>
</p:presentation>"#
            .to_owned()
    }

    fn custom_presentation_rels() -> String {
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rCustomSlide" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/title.xml"/>
</Relationships>"#
            .to_owned()
    }

    fn custom_slide() -> String {
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="2" name="Title"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
      <p:spPr/>
      <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Deck Needle</a:t></a:r></a:p></p:txBody>
    </p:sp>
  </p:spTree></p:cSld>
</p:sld>"#
            .to_owned()
    }
}
