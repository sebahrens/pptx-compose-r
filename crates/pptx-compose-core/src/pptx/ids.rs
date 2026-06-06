use crate::xml::document::{XmlElement, XmlNode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpTreePath {
    pub sp_tree_path: Vec<u32>,
    pub group_path: Vec<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ElementKind {
    Shape,
    Picture,
    Group,
    GraphicFrame,
    Connector,
    Other,
}

impl ElementKind {
    #[must_use]
    pub const fn agent_prefix(self) -> &'static str {
        match self {
            Self::Shape => "shape",
            Self::Picture => "pic",
            Self::Group => "group",
            Self::GraphicFrame => "graphic",
            Self::Connector => "cxn",
            Self::Other => "oth",
        }
    }
}

#[must_use]
pub fn index_sp_tree(sp_tree: &XmlElement) -> Vec<(SpTreePath, ElementKind)> {
    let mut indexed = Vec::new();
    walk_children(sp_tree, &[], &[], &mut indexed);
    indexed
}

#[must_use]
pub fn agent_element_id(slide_id: &str, kind: ElementKind, path: &SpTreePath) -> String {
    let key = dotted_path(&path.sp_tree_path);
    format!("{}:{}-{}", slide_id, kind.agent_prefix(), key)
}

fn walk_children(
    parent: &XmlElement,
    path_prefix: &[u32],
    group_path: &[u32],
    indexed: &mut Vec<(SpTreePath, ElementKind)>,
) {
    for (zero_based_index, child) in parent
        .children
        .iter()
        .filter_map(XmlNode::as_element)
        .enumerate()
    {
        let Ok(child_index) = u32::try_from(zero_based_index + 1) else {
            break;
        };
        let mut sp_tree_path = path_prefix.to_vec();
        sp_tree_path.push(child_index);

        let kind = element_kind(child);
        let path = SpTreePath {
            sp_tree_path: sp_tree_path.clone(),
            group_path: group_path.to_vec(),
        };
        indexed.push((path, kind));

        if kind == ElementKind::Group {
            walk_children(child, &sp_tree_path, &sp_tree_path, indexed);
        }
    }
}

fn element_kind(element: &XmlElement) -> ElementKind {
    match element.name.local_name.as_str() {
        "sp" => ElementKind::Shape,
        "pic" => ElementKind::Picture,
        "grpSp" => ElementKind::Group,
        "graphicFrame" => ElementKind::GraphicFrame,
        "cxnSp" => ElementKind::Connector,
        _ => ElementKind::Other,
    }
}

fn dotted_path(path: &[u32]) -> String {
    let mut key = String::new();
    for (index, component) in path.iter().enumerate() {
        if index > 0 {
            key.push('.');
        }
        key.push_str(&component.to_string());
    }
    key
}

#[cfg(test)]
#[test]
fn sp_tree_indexing() {
    use crate::xml::parser::parse_document;

    let raw = br#"
<p:spTree xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:sp/>
  <p:pic/>
  <p:grpSp>
    <p:pic/>
    <p:extLst/>
    <p:cxnSp/>
  </p:grpSp>
  <p:graphicFrame/>
  <p:extLst/>
</p:spTree>
"#;
    let document = parse_document(raw).expect("fixture parses");
    let sp_tree = document.root_element().expect("fixture root");

    let indexed = index_sp_tree(sp_tree);
    let paths_and_kinds: Vec<_> = indexed
        .iter()
        .map(|(path, kind)| {
            (
                path.sp_tree_path.as_slice(),
                path.group_path.as_slice(),
                *kind,
            )
        })
        .collect();

    assert_eq!(
        paths_and_kinds,
        vec![
            (&[1][..], &[][..], ElementKind::Shape),
            (&[2][..], &[][..], ElementKind::Picture),
            (&[3][..], &[][..], ElementKind::Group),
            (&[3, 1][..], &[3][..], ElementKind::Picture),
            (&[3, 2][..], &[3][..], ElementKind::Other),
            (&[3, 3][..], &[3][..], ElementKind::Connector),
            (&[4][..], &[][..], ElementKind::GraphicFrame),
            (&[5][..], &[][..], ElementKind::Other),
        ]
    );

    let group = &indexed[2];
    assert_eq!(
        agent_element_id("slide-1", group.1, &group.0),
        "slide-1:group-3"
    );

    let nested_picture = &indexed[3];
    assert_eq!(
        agent_element_id("slide-1", nested_picture.1, &nested_picture.0),
        "slide-1:pic-3.1"
    );
}
