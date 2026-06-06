use std::path::PathBuf;

use pptx_compose_core::zip::{limits::ResourceLimits, writer::WriteMode};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OpenOptions {
    pub resource_limits: ResourceLimits,
}

impl OpenOptions {
    #[must_use]
    pub fn with_resource_limits(resource_limits: ResourceLimits) -> Self {
        Self { resource_limits }
    }

    #[must_use]
    pub const fn resource_limits(&self) -> &ResourceLimits {
        &self.resource_limits
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentViewOptions {
    pub mode: pptx_compose_json::agent_view::views::ViewMode,
    pub slide_id: Option<String>,
    pub slide_ids: Vec<String>,
    pub element_id: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

impl AgentViewOptions {
    #[must_use]
    pub fn summary() -> Self {
        Self::default()
    }
}

impl Default for AgentViewOptions {
    fn default() -> Self {
        Self {
            mode: pptx_compose_json::agent_view::views::ViewMode::DeckSummary,
            slide_id: None,
            slide_ids: Vec::new(),
            element_id: None,
            cursor: None,
            limit: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApplyPatchOptions {
    pub dry_run: bool,
    pub validate: bool,
}

impl Default for ApplyPatchOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            validate: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteOptions {
    pub mode: WriteMode,
    pub overwrite: bool,
    pub validate: bool,
    pub atomic: bool,
    pub atomic_temp_path: Option<PathBuf>,
    pub keep_temp: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            mode: WriteMode::Preserve,
            overwrite: false,
            validate: true,
            atomic: true,
            atomic_temp_path: None,
            keep_temp: false,
        }
    }
}
