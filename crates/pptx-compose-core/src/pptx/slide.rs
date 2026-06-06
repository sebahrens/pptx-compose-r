use crate::{
    error::Result,
    opc::{
        package::Package,
        part_name::PartName,
        relationships::{Relationship, RelationshipSet, TargetMode, resolve_internal_target},
    },
};

const SLIDE_LAYOUT_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout";
const SLIDE_MASTER_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster";
const THEME_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlideId(u32);

impl SlideId {
    pub fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(&self) -> u32 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlideLayoutRef {
    pub part_name: PartName,
    pub master: Option<PartName>,
    pub theme: Option<PartName>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Slide {
    pub id: SlideId,
    pub agent_index: u32,
    pub part_name: PartName,
    pub rels_part_name: PartName,
    pub layout: Option<SlideLayoutRef>,
}

pub type SlideRef = Slide;

impl Slide {
    #[must_use]
    pub fn agent_id(&self) -> String {
        format!("slide-{}", self.agent_index + 1)
    }
}

pub(crate) fn rels_part_name_for(part_name: &PartName) -> Result<PartName> {
    let path = part_name.as_str();
    let Some((directory, file_name)) = path.rsplit_once('/') else {
        return PartName::from_zip_entry(format!("/_rels/{path}.rels").as_str());
    };
    let rels_path = if directory.is_empty() {
        format!("/_rels/{file_name}.rels")
    } else {
        format!("{directory}/_rels/{file_name}.rels")
    };
    PartName::from_zip_entry(rels_path.as_str())
}

pub(crate) fn resolve_layout(
    slide_rels: &RelationshipSet,
    package: &Package,
) -> Option<SlideLayoutRef> {
    let layout_part_name = resolve_related_part(slide_rels, SLIDE_LAYOUT_REL_TYPE)?;
    package.parts().get(&layout_part_name)?;

    let master = package
        .relationships()
        .set_for(&layout_part_name)
        .and_then(|layout_rels| resolve_related_part(layout_rels, SLIDE_MASTER_REL_TYPE))
        .filter(|part_name| package.parts().get(part_name).is_some());

    let theme = master
        .as_ref()
        .and_then(|master_part_name| package.relationships().set_for(master_part_name))
        .and_then(|master_rels| resolve_related_part(master_rels, THEME_REL_TYPE))
        .filter(|part_name| package.parts().get(part_name).is_some());

    Some(SlideLayoutRef {
        part_name: layout_part_name,
        master,
        theme,
    })
}

fn resolve_related_part(rels: &RelationshipSet, rel_type: &str) -> Option<PartName> {
    let relationship = rels.rels.iter().find(|relationship| {
        relationship.rel_type == rel_type && relationship.target_mode == TargetMode::Internal
    })?;
    resolve_relationship_part(relationship)
}

fn resolve_relationship_part(relationship: &Relationship) -> Option<PartName> {
    if relationship.target_mode != TargetMode::Internal {
        return None;
    }

    if let Some(target) = &relationship.resolved_target {
        Some(target.clone())
    } else {
        resolve_internal_target(&relationship.source, &relationship.target).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::rels_part_name_for;
    use crate::opc::part_name::PartName;

    #[test]
    fn derives_rels_part_name() {
        let part_name = PartName::from_zip_entry("ppt/slides/slide1.xml").expect("valid part");

        assert_eq!(
            rels_part_name_for(&part_name)
                .expect("rels part derives")
                .as_str(),
            "/ppt/slides/_rels/slide1.xml.rels"
        );
    }
}

#[cfg(test)]
#[test]
fn resolves_layout_chain() {
    use crate::{
        opc::{
            package::Package,
            relationships::{Relationship, RelationshipSource},
        },
        pptx::presentation::PresentationDocument,
    };

    let mut package = Package::new();
    insert(&mut package, "ppt/presentation.xml", presentation_xml());
    insert(&mut package, "ppt/slides/slide1.xml", b"<p:sld/>");
    insert(
        &mut package,
        "ppt/slideLayouts/slideLayout1.xml",
        b"<p:sldLayout/>",
    );
    insert(
        &mut package,
        "ppt/slideMasters/slideMaster1.xml",
        b"<p:sldMaster/>",
    );
    insert(&mut package, "ppt/theme/theme1.xml", b"<a:theme/>");

    let presentation_part = part("ppt/presentation.xml");
    let slide_part = part("ppt/slides/slide1.xml");
    let layout_part = part("ppt/slideLayouts/slideLayout1.xml");
    let master_part = part("ppt/slideMasters/slideMaster1.xml");
    let theme_part = part("ppt/theme/theme1.xml");

    package.push_relationship(Relationship::internal(
        RelationshipSource::Package,
        "rOffice",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
        "ppt/presentation.xml",
    ));
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(presentation_part),
        "rSlide",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide",
        "slides/slide1.xml",
    ));
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(slide_part),
        "rLayout",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout",
        "../slideLayouts/slideLayout1.xml",
    ));
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(layout_part.clone()),
        "rMaster",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster",
        "../slideMasters/slideMaster1.xml",
    ));
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(master_part.clone()),
        "rTheme",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme",
        "../theme/theme1.xml",
    ));

    let document = PresentationDocument::open(package).expect("presentation opens");
    let package = document.package();
    let slide = document.slides().first().expect("slide is present");
    let layout = slide.layout.as_ref().expect("layout resolves");

    assert_eq!(layout.part_name, layout_part);
    assert_eq!(layout.master.as_ref(), Some(&master_part));
    assert_eq!(layout.theme.as_ref(), Some(&theme_part));
    assert!(package.parts().get(&layout.part_name).is_some());
    assert!(
        layout
            .master
            .as_ref()
            .and_then(|part_name| package.parts().get(part_name))
            .is_some()
    );
    assert!(
        layout
            .theme
            .as_ref()
            .and_then(|part_name| package.parts().get(part_name))
            .is_some()
    );
}

#[cfg(test)]
fn presentation_xml() -> &'static [u8] {
    br#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="256" r:id="rSlide"/></p:sldIdLst></p:presentation>"#
}

#[cfg(test)]
fn insert(package: &mut Package, name: &str, bytes: &[u8]) {
    package
        .insert_zip_entry(name, bytes.to_vec())
        .expect("fixture part inserts");
}

#[cfg(test)]
fn part(name: &str) -> PartName {
    PartName::from_zip_entry(name).expect("valid fixture part")
}
