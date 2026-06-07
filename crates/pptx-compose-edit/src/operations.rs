use pptx_compose_core::{
    error::{Error, ErrorCode, Result},
    opc::part_name::PartName,
    pptx::ids::ElementKind,
    xml::{
        document::{XmlElement, XmlNode},
        namespaces::NamespaceBinding,
    },
};

use crate::patch::{InsertOptions, ZOrder, ZOrderKeyword};

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

pub(super) fn insert_shape_tree_child(
    sp_tree: &mut XmlElement,
    child: XmlElement,
    insert: Option<&InsertOptions>,
) -> Result<u32> {
    let child_index = shape_tree_child_index(sp_tree, insert)?;
    let path_component = element_ordinal_before_child_index(sp_tree, child_index)
        .checked_add(1)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                "Shape tree has too many elements to assign an agent element id.",
            )
        })?;
    sp_tree
        .children
        .insert(child_index, XmlNode::Element(child));
    Ok(path_component)
}

fn shape_tree_child_index(sp_tree: &XmlElement, insert: Option<&InsertOptions>) -> Result<usize> {
    match insert.and_then(|insert| insert.z_order) {
        None | Some(ZOrder::Keyword(ZOrderKeyword::Front)) => Ok(front_child_index(sp_tree)),
        Some(ZOrder::Keyword(ZOrderKeyword::Back)) => Ok(back_child_index(sp_tree)),
        Some(ZOrder::Index(index)) => explicit_child_index(sp_tree, index),
    }
}

fn explicit_child_index(sp_tree: &XmlElement, index: u32) -> Result<usize> {
    let back_ordinal = element_ordinal_before_child_index(sp_tree, back_child_index(sp_tree))
        .checked_add(1)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                "Shape tree has too many elements to assign an agent element id.",
            )
        })?;
    let front_ordinal = element_ordinal_before_child_index(sp_tree, front_child_index(sp_tree))
        .checked_add(1)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                "Shape tree has too many elements to assign an agent element id.",
            )
        })?;

    if index < back_ordinal || index > front_ordinal {
        return Err(Error::new(
            ErrorCode::InvalidInput,
            format!(
                "insert.z_order index must be between {back_ordinal} and {front_ordinal} for this shape tree."
            ),
        ));
    }

    child_index_for_element_ordinal(sp_tree, index)
}

fn front_child_index(sp_tree: &XmlElement) -> usize {
    sp_tree
        .children
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, child)| {
            child
                .as_element()
                .filter(|element| is_real_shape_tree_child(element))
                .map(|_| index + 1)
        })
        .unwrap_or_else(|| back_child_index(sp_tree))
}

fn back_child_index(sp_tree: &XmlElement) -> usize {
    sp_tree
        .children
        .iter()
        .position(|child| child.as_element().is_some_and(is_real_shape_tree_child))
        .unwrap_or_else(|| {
            sp_tree
                .children
                .iter()
                .enumerate()
                .filter_map(|(index, child)| {
                    child.as_element().and_then(|element| {
                        matches!(element.name.local_name.as_str(), "nvGrpSpPr" | "grpSpPr")
                            .then_some(index + 1)
                    })
                })
                .max()
                .unwrap_or(sp_tree.children.len())
        })
}

fn child_index_for_element_ordinal(sp_tree: &XmlElement, ordinal: u32) -> Result<usize> {
    let mut element_ordinal = 0_u32;
    for (child_index, child) in sp_tree.children.iter().enumerate() {
        if child.as_element().is_some() {
            element_ordinal = element_ordinal.checked_add(1).ok_or_else(|| {
                Error::new(
                    ErrorCode::UnsupportedEdit,
                    "Shape tree has too many elements to assign an agent element id.",
                )
            })?;
            if element_ordinal == ordinal {
                return Ok(child_index);
            }
        }
    }
    Ok(sp_tree.children.len())
}

fn element_ordinal_before_child_index(sp_tree: &XmlElement, child_index: usize) -> u32 {
    sp_tree
        .children
        .iter()
        .take(child_index)
        .filter(|child| child.as_element().is_some())
        .count()
        .try_into()
        .unwrap_or(u32::MAX)
}

fn is_real_shape_tree_child(element: &XmlElement) -> bool {
    matches!(
        element.name.local_name.as_str(),
        "sp" | "pic" | "grpSp" | "graphicFrame" | "cxnSp"
    )
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
