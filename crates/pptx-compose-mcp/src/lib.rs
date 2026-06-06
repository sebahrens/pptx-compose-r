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

use sessions::SessionStore;

const RAW_GET_PART_XML: &str = "pptx_get_part_xml";
const RAW_REPLACE_PART_XML: &str = "pptx_replace_part_xml";

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
    pub content_type: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyPatchInput {
    pub session_id: String,
    pub expected_revision: u64,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidatePatchInput {
    pub session_id: String,
    pub expected_revision: u64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloseInput {
    pub session_id: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportInput {
    pub output_path: Option<String>,
    #[serde(default)]
    pub overwrite: bool,
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
    pub async fn pptx_get_document_summary(&self) -> rmcp::Json<outputs::DocumentSummaryOutput> {
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
    pub async fn pptx_list_slides(&self) -> rmcp::Json<outputs::ListSlidesOutput> {
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
    pub async fn pptx_get_slide(&self) -> rmcp::Json<outputs::SlideOutput> {
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
    pub async fn pptx_list_elements(&self) -> rmcp::Json<outputs::ListElementsOutput> {
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
    pub async fn pptx_get_element(&self) -> rmcp::Json<outputs::ElementOutput> {
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
    pub async fn pptx_find_text(&self) -> rmcp::Json<outputs::FindTextOutput> {
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
        if let Some(media_path) = input.0.media_path {
            self.permission_policy
                .check_read(&media_path)
                .map_err(|error| outputs::map_error(error.into_core_error()))?;
            let handle = self
                .sessions
                .import_media_path(&input.0.session_id, media_path, &input.0.content_type)
                .map_err(outputs::map_error)?;
            return Ok(Json(outputs::ImportMediaOutput::imported(handle)));
        }

        Err(outputs::map_error(pptx_compose::core::error::Error::new(
            pptx_compose::core::error::ErrorCode::InvalidInput,
            "pptx_import_media requires media_path.",
        )))
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
            .check_revision(&input.0.session_id, input.0.expected_revision)
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
        let revision = self
            .sessions
            .record_apply(
                &input.0.session_id,
                input.0.expected_revision,
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
