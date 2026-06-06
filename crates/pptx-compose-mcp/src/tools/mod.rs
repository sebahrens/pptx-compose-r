use std::collections::BTreeSet;

use rmcp::model::Tool;

use crate::PptxServer;

pub const DEFAULT_TOOL_NAMES: &[&str] = &[
    "pptx_open",
    "pptx_get_document_summary",
    "pptx_list_slides",
    "pptx_get_slide",
    "pptx_list_elements",
    "pptx_get_element",
    "pptx_find_text",
    "pptx_import_media",
    "pptx_validate_patch",
    "pptx_apply_patch",
    "pptx_validate",
    "pptx_export",
    "pptx_close",
];

pub const RAW_TOOL_NAMES: &[&str] = &["pptx_get_part_xml", "pptx_replace_part_xml"];

pub fn exposed_tools(server: &PptxServer) -> Vec<Tool> {
    server.tool_routes().list_all()
}

pub fn exposed_tool_names(server: &PptxServer) -> BTreeSet<String> {
    exposed_tools(server)
        .into_iter()
        .map(|tool| tool.name.into_owned())
        .collect()
}

#[cfg(test)]
pub fn assert_raw_tools_disabled_by_default() {
    use crate::{PptxServer, ServerConfig};

    let default_server = PptxServer::default();
    let default_tools = exposed_tool_names(&default_server);

    assert_eq!(default_tools.len(), DEFAULT_TOOL_NAMES.len());
    for name in DEFAULT_TOOL_NAMES {
        assert!(default_tools.contains(*name), "missing default tool {name}");
    }
    for name in RAW_TOOL_NAMES {
        assert!(
            !default_tools.contains(*name),
            "raw tool exposed by default: {name}"
        );
    }

    let raw_enabled_server = PptxServer::with_config(ServerConfig {
        enable_raw_xml_tools: true,
    });
    let raw_enabled_tools = exposed_tool_names(&raw_enabled_server);

    assert_eq!(
        raw_enabled_tools.len(),
        DEFAULT_TOOL_NAMES.len() + RAW_TOOL_NAMES.len()
    );
    for name in DEFAULT_TOOL_NAMES.iter().chain(RAW_TOOL_NAMES.iter()) {
        assert!(
            raw_enabled_tools.contains(*name),
            "missing raw-enabled tool {name}"
        );
    }
}

#[test]
fn raw_tools_disabled_by_default() {
    assert_raw_tools_disabled_by_default();
}
