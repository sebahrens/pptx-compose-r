#![deny(warnings)]

use rmcp::{
    ServerHandler, ServiceExt, handler::server::router::tool::ToolRouter, model::ServerInfo, tool,
    tool_handler, tool_router,
};

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
        Self::with_session_store(SessionStore, config)
    }

    pub fn with_session_store(sessions: SessionStore, config: ServerConfig) -> Self {
        let mut tool_router = Self::build_tool_router();
        if !config.enable_raw_xml_tools {
            tool_router.disable_route(RAW_GET_PART_XML);
            tool_router.disable_route(RAW_REPLACE_PART_XML);
        }

        Self {
            sessions,
            config,
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
    pub async fn pptx_open(&self) -> rmcp::Json<outputs::OpenOutput> {
        rmcp::Json(outputs::OpenOutput::stub("pptx_open"))
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
    pub async fn pptx_import_media(&self) -> rmcp::Json<outputs::ImportMediaOutput> {
        rmcp::Json(outputs::ImportMediaOutput::stub("pptx_import_media"))
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
    pub async fn pptx_validate_patch(&self) -> rmcp::Json<outputs::ValidatePatchOutput> {
        rmcp::Json(outputs::ValidatePatchOutput::stub("pptx_validate_patch"))
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
    pub async fn pptx_apply_patch(&self) -> rmcp::Json<outputs::ApplyPatchOutput> {
        rmcp::Json(outputs::ApplyPatchOutput::stub("pptx_apply_patch"))
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
    pub async fn pptx_export(&self) -> rmcp::Json<outputs::ExportOutput> {
        rmcp::Json(outputs::ExportOutput::stub("pptx_export"))
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
    pub async fn pptx_close(&self) -> rmcp::Json<outputs::CloseOutput> {
        rmcp::Json(outputs::CloseOutput::stub("pptx_close"))
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
    let service = PptxServer::default()
        .serve(rmcp::transport::io::stdio())
        .await?;
    service
        .waiting()
        .await
        .map(|_| ())
        .map_err(|error| rmcp::service::ServerInitializeError::ConnectionClosed(error.to_string()))
}
