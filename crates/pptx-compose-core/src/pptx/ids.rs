use crate::xml::document::{XmlElement, XmlNode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpTreePath {
    pub sp_tree_path: Vec<u32>,
    pub group_path: Vec<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ElementKind {
    TextBox,
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
            Self::TextBox | Self::Shape => "shape",
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
pub fn slide_agent_id(presentation_order_index: usize) -> String {
    format!("slide-{}", presentation_order_index + 1)
}

#[must_use]
pub fn agent_element_id(slide_id: &str, kind: ElementKind, path: &SpTreePath) -> String {
    let key = dotted_path(&path.sp_tree_path);
    format!("{}:{}-{}", slide_id, kind.agent_prefix(), key)
}

#[must_use]
pub fn paragraph_agent_id(element_id: &str, p_index: usize) -> String {
    format!("{element_id}:p{p_index}")
}

#[must_use]
pub fn run_agent_id(paragraph_id: &str, r_index: usize) -> String {
    format!("{paragraph_id}:r{r_index}")
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
        "sp" if is_text_box(element) => ElementKind::TextBox,
        "sp" => ElementKind::Shape,
        "pic" => ElementKind::Picture,
        "grpSp" => ElementKind::Group,
        "graphicFrame" => ElementKind::GraphicFrame,
        "cxnSp" => ElementKind::Connector,
        _ => ElementKind::Other,
    }
}

fn is_text_box(element: &XmlElement) -> bool {
    first_descendant(element, "cNvSpPr")
        .and_then(|element| attr(element, "txBox"))
        .is_some_and(parse_bool)
}

fn first_descendant<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a XmlElement> {
    for child in element.children.iter().filter_map(XmlNode::as_element) {
        if child.name.local_name == local_name {
            return Some(child);
        }
        if let Some(descendant) = first_descendant(child, local_name) {
            return Some(descendant);
        }
    }
    None
}

fn attr<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local_name == local_name)
        .map(|attribute| attribute.value.as_str())
}

fn parse_bool(value: &str) -> bool {
    matches!(value, "1" | "true" | "True" | "TRUE")
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

#[cfg(test)]
#[test]
fn agent_id_derivation() {
    let slide_id = slide_agent_id(1);
    assert_eq!(slide_id, "slide-2");

    let element_path = SpTreePath {
        sp_tree_path: vec![4],
        group_path: Vec::new(),
    };
    let element_id = agent_element_id(&slide_id, ElementKind::Shape, &element_path);
    assert_eq!(element_id, "slide-2:shape-4");

    let paragraph_id = paragraph_agent_id(&element_id, 0);
    assert_eq!(paragraph_id, "slide-2:shape-4:p0");

    let child_names = ["r", "br", "r"];
    let second_run_index = child_names
        .iter()
        .filter(|name| **name == "r")
        .enumerate()
        .find_map(|(run_index, name)| (*name == "r" && run_index == 1).then_some(run_index))
        .expect("fixture contains second run");
    let run_id = run_agent_id(&paragraph_id, second_run_index);
    assert_eq!(run_id, "slide-2:shape-4:p0:r1");
}
