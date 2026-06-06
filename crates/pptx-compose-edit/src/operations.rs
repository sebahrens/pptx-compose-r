use pptx_compose_core::{
    opc::part_name::PartName,
    pptx::ids::ElementKind,
    xml::{document::XmlElement, namespaces::NamespaceBinding},
};

pub mod add_image;
pub mod add_text_box;
mod bounds;
pub mod move_resize;
pub mod replace_image;
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

const P_NS: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

pub(super) fn ensure_slide_namespaces(root: &mut XmlElement) {
    ensure_namespace(root, Some("p"), P_NS);
    ensure_namespace(root, Some("a"), A_NS);
    ensure_namespace(root, Some("r"), R_NS);
}

fn ensure_namespace(root: &mut XmlElement, prefix: Option<&str>, uri: &str) {
    if root.namespaces.resolve_prefix(prefix) == Some(uri) {
        return;
    }
    if let Some(prefix) = prefix {
        root.namespaces
            .push(NamespaceBinding::prefixed(prefix, uri));
    } else {
        root.namespaces.push(NamespaceBinding::default(uri));
    }
}
