use pptx_compose_core::{opc::part_name::PartName, pptx::ids::ElementKind};

pub mod add_text_box;
pub mod move_resize;
pub mod replace_text;
pub mod set_alt_text;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolvedTarget {
    Slide(ResolvedSlide),
    Element(ResolvedElement),
    MediaPart(ResolvedMediaPart),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedSlide {
    pub slide_id: String,
    pub part: PartName,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedElement {
    pub slide_id: String,
    pub element_id: String,
    pub kind: ElementKind,
    pub part: PartName,
    pub sp_tree_path: Vec<u32>,
    pub group_path: Vec<u32>,
    pub cnvpr_id: Option<i64>,
    pub text_hash: Option<String>,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedMediaPart {
    pub part: PartName,
}
