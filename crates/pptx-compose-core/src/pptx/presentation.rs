use crate::{
    error::{Error, Result},
    opc::{
        package::{Package, SlideIdEntry},
        part_name::PartName,
        relationships::{Relationship, RelationshipSource, TargetMode, resolve_internal_target},
    },
    pptx::slide::{Slide, SlideId, SlideRef, rels_part_name_for, resolve_layout},
    xml::{document::XmlElement, parser::parse_document},
};

const OFFICE_DOCUMENT_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const SLIDE_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresentationDocument {
    package: Package,
    presentation: Presentation,
}

impl PresentationDocument {
    pub fn open(package: Package) -> Result<Self> {
        let presentation = Presentation::open(&package)?;
        Ok(Self {
            package,
            presentation,
        })
    }

    #[must_use]
    pub const fn package(&self) -> &Package {
        &self.package
    }

    #[must_use]
    pub const fn presentation(&self) -> &Presentation {
        &self.presentation
    }

    #[must_use]
    pub fn slides(&self) -> &[SlideRef] {
        self.presentation.slides()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Presentation {
    pub part_name: PartName,
    slides: Vec<SlideRef>,
    pub metadata: PresentationMetadata,
}

impl Presentation {
    fn open(package: &Package) -> Result<Self> {
        let part_name = locate_presentation_part(package)?;
        let raw = package
            .parts()
            .get(&part_name)
            .ok_or_else(|| {
                Error::unsupported_package(format!(
                    "Presentation relationship points at missing part {part_name}."
                ))
            })?
            .bytes();
        let document = parse_document(raw).map_err(|source| {
            Error::with_source(
                source.code(),
                format!("Could not parse presentation part {part_name}."),
                source,
            )
        })?;
        let root = document.root_element().ok_or_else(|| {
            Error::unsupported_package(format!(
                "Presentation part {part_name} has no root element."
            ))
        })?;

        let slide_id_list = child_element(root, "sldIdLst").ok_or_else(|| {
            Error::unsupported_package(format!(
                "Presentation part {part_name} is missing p:sldIdLst."
            ))
        })?;

        let mut slides = Vec::new();
        for child in slide_id_list
            .children
            .iter()
            .filter_map(|node| node.as_element())
        {
            if child.name.local_name != "sldId" {
                continue;
            }
            let id = parse_slide_id(required_attr(child, "id")?)?;
            let relationship_id = required_prefixed_attr(child, Some("r"), "id")?;
            let slide_part_name = resolve_slide_part(package, &part_name, relationship_id)?;
            let agent_index = u32::try_from(slides.len()).map_err(|_| {
                Error::resource_limit_exceeded("Presentation contains too many slides.")
            })?;
            let layout = package
                .relationships()
                .set_for(&slide_part_name)
                .and_then(|slide_rels| resolve_layout(slide_rels, package));
            slides.push(Slide {
                id,
                agent_index,
                rels_part_name: rels_part_name_for(&slide_part_name)?,
                part_name: slide_part_name,
                layout,
            });
        }

        Ok(Self {
            part_name,
            slides,
            metadata: PresentationMetadata::default(),
        })
    }

    #[must_use]
    pub fn slides(&self) -> &[SlideRef] {
        &self.slides
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PresentationMetadata {}

pub fn hydrate_package_slide_ids(package: &mut Package) {
    let slide_ids = read_package_slide_ids(package).unwrap_or_default();
    package.replace_slide_ids(slide_ids);
}

fn read_package_slide_ids(package: &Package) -> Result<Vec<SlideIdEntry>> {
    let part_name = locate_presentation_part(package)?;
    let Some(part) = package.parts().get(&part_name) else {
        return Ok(Vec::new());
    };
    let document = parse_document(part.bytes())?;
    let Some(root) = document.root_element() else {
        return Ok(Vec::new());
    };
    let Some(slide_id_list) = child_element(root, "sldIdLst") else {
        return Ok(Vec::new());
    };

    let mut slide_ids = Vec::new();
    for child in slide_id_list
        .children
        .iter()
        .filter_map(|node| node.as_element())
    {
        if child.name.local_name != "sldId" {
            continue;
        }
        let Some(slide_id) = optional_attr(child, "id") else {
            continue;
        };
        let relationship_id = optional_prefixed_attr(child, Some("r"), "id").map(str::to_owned);
        let part = relationship_id.as_deref().and_then(|relationship_id| {
            resolve_slide_part(package, &part_name, relationship_id).ok()
        });
        slide_ids.push(SlideIdEntry {
            slide_id: slide_id.to_owned(),
            relationship_id,
            part,
        });
    }
    Ok(slide_ids)
}

fn locate_presentation_part(package: &Package) -> Result<PartName> {
    let relationship = package
        .relationships()
        .iter()
        .find(|relationship| {
            relationship.source == RelationshipSource::Package
                && relationship.target_mode == TargetMode::Internal
                && relationship.rel_type == OFFICE_DOCUMENT_REL_TYPE
        })
        .ok_or_else(|| {
            Error::unsupported_package(
                "Package root relationships do not contain an Office document relationship.",
            )
        })?;

    resolve_relationship_target(relationship)
}

fn resolve_slide_part(
    package: &Package,
    presentation_part_name: &PartName,
    relationship_id: &str,
) -> Result<PartName> {
    let relationship = package
        .relationships()
        .iter()
        .find(|relationship| {
            relationship.source == RelationshipSource::Part(presentation_part_name.clone())
                && relationship.id == relationship_id
        })
        .ok_or_else(|| {
            Error::unsupported_package(format!(
                "Presentation slide relationship {relationship_id} is missing."
            ))
        })?;

    if relationship.rel_type != SLIDE_REL_TYPE {
        return Err(Error::unsupported_package(format!(
            "Presentation relationship {relationship_id} is not a slide relationship."
        )));
    }

    resolve_relationship_target(relationship)
}

fn resolve_relationship_target(relationship: &Relationship) -> Result<PartName> {
    if relationship.target_mode != TargetMode::Internal {
        return Err(Error::unsupported_package(format!(
            "Relationship {} must be internal.",
            relationship.id
        )));
    }

    if let Some(target) = &relationship.resolved_target {
        Ok(target.clone())
    } else {
        resolve_internal_target(&relationship.source, &relationship.target)
    }
}

fn child_element<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a XmlElement> {
    element
        .children
        .iter()
        .filter_map(|node| node.as_element())
        .find(|child| child.name.local_name == local_name)
}

fn parse_slide_id(value: &str) -> Result<SlideId> {
    let parsed = value.parse::<u32>().map_err(|source| {
        Error::with_source(
            crate::error::ErrorCode::UnsupportedPackage,
            format!("Slide id {value} is not a valid unsigned integer."),
            source,
        )
    })?;
    Ok(SlideId::new(parsed))
}

fn required_attr<'a>(element: &'a XmlElement, local_name: &str) -> Result<&'a str> {
    optional_attr(element, local_name).ok_or_else(|| {
        Error::unsupported_package(format!(
            "Element {} is missing required attribute {local_name}.",
            element.name.raw
        ))
    })
}

fn optional_attr<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local_name == local_name)
        .map(|attribute| attribute.value.as_str())
}

fn required_prefixed_attr<'a>(
    element: &'a XmlElement,
    prefix: Option<&str>,
    local_name: &str,
) -> Result<&'a str> {
    optional_prefixed_attr(element, prefix, local_name).ok_or_else(|| {
        let qualified_name = prefix.map_or_else(
            || local_name.to_owned(),
            |prefix| format!("{prefix}:{local_name}"),
        );
        Error::unsupported_package(format!(
            "Element {} is missing required attribute {qualified_name}.",
            element.name.raw
        ))
    })
}

fn optional_prefixed_attr<'a>(
    element: &'a XmlElement,
    prefix: Option<&str>,
    local_name: &str,
) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| {
            attribute.name.prefix.as_deref() == prefix && attribute.name.local_name == local_name
        })
        .map(|attribute| attribute.value.as_str())
}

#[cfg(test)]
#[test]
fn slide_order_from_sldidlst() {
    let mut package = Package::new();
    insert(&mut package, "custom/slides/slide3.xml", b"<p:sld/>");
    insert(&mut package, "custom/slides/slide1.xml", b"<p:sld/>");
    insert(&mut package, "custom/presentation.xml", presentation_xml());
    insert(&mut package, "custom/slides/slide2.xml", b"<p:sld/>");

    let presentation_part = part("custom/presentation.xml");
    package.push_relationship(Relationship::internal(
        RelationshipSource::Package,
        "rOffice",
        OFFICE_DOCUMENT_REL_TYPE,
        "custom/presentation.xml",
    ));
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(presentation_part.clone()),
        "rSlideA",
        SLIDE_REL_TYPE,
        "slides/slide3.xml",
    ));
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(presentation_part.clone()),
        "rSlideB",
        SLIDE_REL_TYPE,
        "slides/slide1.xml",
    ));
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(presentation_part),
        "rSlideC",
        SLIDE_REL_TYPE,
        "slides/slide2.xml",
    ));

    let document = PresentationDocument::open(package).expect("presentation opens");
    let slides = document.slides();

    assert_eq!(slides.len(), 3);
    assert_eq!(slides[0].agent_id(), "slide-1");
    assert_eq!(slides[0].id.value(), 300);
    assert_eq!(slides[0].part_name.as_str(), "/custom/slides/slide3.xml");
    assert_eq!(
        slides[0].rels_part_name.as_str(),
        "/custom/slides/_rels/slide3.xml.rels"
    );
    assert_eq!(slides[1].agent_id(), "slide-2");
    assert_eq!(slides[1].id.value(), 100);
    assert_eq!(slides[1].part_name.as_str(), "/custom/slides/slide1.xml");
    assert_eq!(slides[2].agent_id(), "slide-3");
    assert_eq!(slides[2].id.value(), 200);
    assert_eq!(slides[2].part_name.as_str(), "/custom/slides/slide2.xml");
}

#[cfg(test)]
#[test]
fn hydrates_package_slide_ids_from_sldidlst() {
    let mut package = Package::new();
    insert(&mut package, "custom/slides/slide3.xml", b"<p:sld/>");
    insert(&mut package, "custom/slides/slide1.xml", b"<p:sld/>");
    insert(&mut package, "custom/presentation.xml", presentation_xml());

    let presentation_part = part("custom/presentation.xml");
    package.push_relationship(Relationship::internal(
        RelationshipSource::Package,
        "rOffice",
        OFFICE_DOCUMENT_REL_TYPE,
        "custom/presentation.xml",
    ));
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(presentation_part.clone()),
        "rSlideA",
        SLIDE_REL_TYPE,
        "slides/slide3.xml",
    ));
    package.push_relationship(Relationship::internal(
        RelationshipSource::Part(presentation_part),
        "rSlideB",
        SLIDE_REL_TYPE,
        "slides/slide1.xml",
    ));

    hydrate_package_slide_ids(&mut package);

    assert_eq!(package.slide_ids().len(), 3);
    assert_eq!(package.slide_ids()[0].slide_id, "300");
    assert_eq!(
        package.slide_ids()[0].part.as_ref().map(PartName::as_str),
        Some("/custom/slides/slide3.xml")
    );
    assert_eq!(package.slide_ids()[1].slide_id, "100");
    assert_eq!(
        package.slide_ids()[1].relationship_id.as_deref(),
        Some("rSlideB")
    );
    assert_eq!(package.slide_ids()[2].slide_id, "200");
    assert_eq!(package.slide_ids()[2].part, None);
}

#[cfg(test)]
fn presentation_xml() -> &'static [u8] {
    br#"<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="300" r:id="rSlideA"/><p:sldId id="100" r:id="rSlideB"/><p:sldId id="200" r:id="rSlideC"/></p:sldIdLst></p:presentation>"#
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
