use crate::{error::Result, opc::part_name::PartName};

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
