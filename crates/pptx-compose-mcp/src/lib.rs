#![deny(warnings)]

use rmcp::{
    Json, ServerHandler, ServiceExt, handler::server::router::tool::ToolRouter, model::ServerInfo,
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;

pub mod outputs;
pub mod permissions;
pub mod prompts;
pub mod resources;
pub mod sessions;
pub mod tools;

use pptx_compose::edit::patch::Patch;
use sessions::SessionStore;

const RAW_GET_PART_XML: &str = "pptx_get_part_xml";
const RAW_REPLACE_PART_XML: &str = "pptx_replace_part_xml";
const MAX_INLINE_MEDIA_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ServerConfig {
    pub enable_raw_xml_tools: bool,
}

#[derive(Clone, Debug)]
pub struct PptxServer {
    sessions: SessionStore,
    config: ServerConfig,
    permission_policy: permissions::PermissionPolicy,
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
        let mut tool_router = Self::build_tool_router();
        if !config.enable_raw_xml_tools {
            tool_router.disable_route(RAW_GET_PART_XML);
            tool_router.disable_route(RAW_REPLACE_PART_XML);
        }

        Self {
            sessions,
            config,
            permission_policy,
            tool_router,
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
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenInput {
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportMediaInput {
    pub session_id: String,
    pub media_path: Option<String>,
    pub inline: Option<InlineMediaInput>,
    pub content_type: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InlineMediaInput {
    pub encoding: String,
    pub data: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyPatchInput {
    pub session_id: String,
    pub patch: Patch,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidatePatchInput {
    pub session_id: String,
    pub patch: Patch,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloseInput {
    pub session_id: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportInput {
    pub session_id: String,
    pub expected_revision: Option<u64>,
    pub output_path: Option<String>,
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SummaryInput {
    pub session_id: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListSlidesInput {
    pub session_id: String,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetSlideInput {
    pub session_id: String,
    pub slide_id: String,
    pub expected_revision: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListElementsInput {
    pub session_id: String,
    pub slide_id: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetElementInput {
    pub session_id: String,
    pub element_id: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindTextInput {
    pub session_id: String,
    pub query: String,
    pub scope: String,
    pub cursor: Option<String>,
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
        self.permission_policy
            .check_read(&input.0.path)
            .map_err(|error| outputs::map_error(error.into_core_error()))?;

        let opened = self
            .sessions
            .open_path(&input.0.path)
            .map_err(outputs::map_error)?;
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
        _input: rmcp::handler::server::wrapper::Parameters<SummaryInput>,
    ) -> rmcp::Json<outputs::DocumentSummaryOutput> {
        rmcp::Json(outputs::DocumentSummaryOutput::stub(
            "pptx_get_document_summary",
        ))
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
        _input: rmcp::handler::server::wrapper::Parameters<ListSlidesInput>,
    ) -> rmcp::Json<outputs::ListSlidesOutput> {
        rmcp::Json(outputs::ListSlidesOutput::stub("pptx_list_slides"))
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
        _input: rmcp::handler::server::wrapper::Parameters<GetSlideInput>,
    ) -> rmcp::Json<outputs::SlideOutput> {
        rmcp::Json(outputs::SlideOutput::stub("pptx_get_slide"))
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
        _input: rmcp::handler::server::wrapper::Parameters<ListElementsInput>,
    ) -> rmcp::Json<outputs::ListElementsOutput> {
        rmcp::Json(outputs::ListElementsOutput::stub("pptx_list_elements"))
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
        _input: rmcp::handler::server::wrapper::Parameters<GetElementInput>,
    ) -> rmcp::Json<outputs::ElementOutput> {
        rmcp::Json(outputs::ElementOutput::stub("pptx_get_element"))
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
        _input: rmcp::handler::server::wrapper::Parameters<FindTextInput>,
    ) -> rmcp::Json<outputs::FindTextOutput> {
        rmcp::Json(outputs::FindTextOutput::stub("pptx_find_text"))
    }

    /// Stage media bytes or a path as a media_ref.
    #[tool(
        name = "pptx_import_media",
        annotations(
            title = "Import Media",
            read_only_hint = false,
            destructive_hint = false,
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
                self.permission_policy
                    .check_read(&media_path)
                    .map_err(|error| outputs::map_error(error.into_core_error()))?;
                let handle = self
                    .sessions
                    .import_media_path(&input.session_id, media_path, &input.content_type)
                    .map_err(outputs::map_error)?;
                Ok(Json(outputs::ImportMediaOutput::imported(handle)))
            }
            (None, Some(inline)) => {
                let bytes = decode_inline_media(&inline).map_err(outputs::map_error)?;
                let handle = self
                    .sessions
                    .import_media_bytes(&input.session_id, bytes, &input.content_type)
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
            .check_patch_envelope(&input.0.session_id, &input.0.patch)
            .map_err(outputs::map_error)?;
        Ok(rmcp::Json(outputs::ValidatePatchOutput::stub(
            "pptx_validate_patch",
        )))
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
        self.sessions
            .check_patch_envelope(&input.0.session_id, &input.0.patch)
            .map_err(outputs::map_error)?;
        let revision = self
            .sessions
            .record_apply(
                &input.0.session_id,
                u64::from(input.0.patch.base_revision),
                input.0.dry_run,
                true,
            )
            .map_err(outputs::map_error)?;
        Ok(Json(outputs::ApplyPatchOutput::applied(
            &input.0.session_id,
            revision,
            input.0.dry_run,
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
    pub async fn pptx_validate(&self) -> rmcp::Json<outputs::ValidateOutput> {
        rmcp::Json(outputs::ValidateOutput::stub("pptx_validate"))
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
        if let Some(output_path) = input.0.output_path {
            self.permission_policy
                .check_write_with_overwrite(output_path, input.0.overwrite)
                .map_err(|error| outputs::map_error(error.into_core_error()))?;
        }

        Ok(Json(outputs::ExportOutput::stub("pptx_export")))
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

    /// Return raw XML for an OPC part.
    #[tool(
        name = "pptx_get_part_xml",
        annotations(
            title = "Get Raw Part XML",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub async fn pptx_get_part_xml(&self) -> rmcp::Json<outputs::RawPartXmlOutput> {
        rmcp::Json(outputs::RawPartXmlOutput::stub("pptx_get_part_xml"))
    }

    /// Replace raw XML for an OPC part.
    #[tool(
        name = "pptx_replace_part_xml",
        annotations(
            title = "Replace Raw Part XML",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn pptx_replace_part_xml(&self) -> rmcp::Json<outputs::ReplacePartXmlOutput> {
        rmcp::Json(outputs::ReplacePartXmlOutput::stub("pptx_replace_part_xml"))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for PptxServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_server_info(rmcp::model::Implementation::new(
            "pptx-compose-mcp",
            env!("CARGO_PKG_VERSION"),
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
    use pptx_compose::core::error::ErrorCode;

    use super::*;

    const ONE_BY_ONE_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=";

    #[tokio::test]
    async fn import_media_accepts_inline_base64_without_filesystem_access() {
        let server = PptxServer::default();
        let opened = open_fixture_session(&server);

        let imported = server
            .pptx_import_media(rmcp::handler::server::wrapper::Parameters(
                ImportMediaInput {
                    session_id: opened.session_id.clone(),
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
    async fn import_media_rejects_inline_content_type_mismatch() {
        let server = PptxServer::default();
        let opened = open_fixture_session(&server);

        let result = server
            .pptx_import_media(rmcp::handler::server::wrapper::Parameters(
                ImportMediaInput {
                    session_id: opened.session_id,
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
            &["session_id"],
            &["slide_id", "cursor", "limit"],
        );
        assert_tool_schema_fields("pptx_get_element", &["session_id", "element_id"], &[]);
        assert_tool_schema_fields(
            "pptx_find_text",
            &["session_id", "query", "scope"],
            &["cursor", "limit"],
        );
        assert_tool_schema_fields(
            "pptx_export",
            &["session_id"],
            &["expected_revision", "output_path", "overwrite"],
        );
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
}
