use pptx_compose_core::zip::{limits::ResourceLimits, writer::WriteMode};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OpenOptions {
    pub resource_limits: ResourceLimits,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentViewOptions {
    pub mode: pptx_compose_json::agent_view::views::ViewMode,
    pub slide_id: Option<String>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriteOptions {
    pub mode: WriteMode,
    pub overwrite: bool,
    pub validate: bool,
    pub atomic: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            mode: WriteMode::Preserve,
            overwrite: false,
            validate: true,
            atomic: true,
        }
    }
}
