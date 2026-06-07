use pptx_compose_core::{
    error::{Error, ErrorCode, ErrorLocation, Result},
    opc::{
        package::Package,
        part_name::PartName,
        relationships::{TargetMode, resolve_internal_target},
    },
    pptx::{
        ids::ElementKind,
        media::{IMAGE_REL_TYPE, next_media_part_name},
    },
    xml::{
        document::{XmlAttribute, XmlDocument, XmlElement, XmlNode},
        parser::parse_document,
        writer::{WriteMode, WriteOptions, write_document},
    },
};
use pptx_compose_json::schemas::OperationTarget;

use crate::{
    media_inputs::MediaInputs,
    operations::{ResolvedElement, add_image::ensure_content_type, is_real_shape_tree_child},
    patch::{PatchEffects, ReplaceImageOperation},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplaceImage {
    pub operation_id: String,
    pub element_id: String,
    pub media_ref: String,
    pub content_type: String,
    pub allow_shared_mutation: bool,
}

impl From<&ReplaceImageOperation> for ReplaceImage {
    fn from(operation: &ReplaceImageOperation) -> Self {
        Self {
            operation_id: operation.operation_id.clone(),
            element_id: operation.target_element_id().to_owned(),
            media_ref: operation.media_ref.clone(),
            content_type: operation.content_type.clone(),
            allow_shared_mutation: operation.allow_shared_mutation.unwrap_or(false),
        }
    }
}

impl ReplaceImage {
    pub fn validate(
        &self,
        package: &Package,
        target: &ResolvedElement,
        media_inputs: &MediaInputs,
    ) -> Result<()> {
        self.validate_target(target)?;
        let resolved = media_inputs
            .resolve(&self.media_ref)
            .map_err(|error| error.with_location(self.location(Some(target), None)))?;
        if resolved.content_type != self.content_type {
            return Err(Error::new(
                ErrorCode::UnsupportedMediaType,
                format!(
                    "replace_image content_type `{}` does not match bound media_ref `{}` content type `{}`.",
                    self.content_type, self.media_ref, resolved.content_type
                ),
            )
            .with_location(self.location(Some(target), None)));
        }

        let picture = self.inspect_picture(package, target)?;
        if picture.link_rel_id.is_some() {
            return Err(Error::new(
                ErrorCode::UnsupportedEdit,
                "replace_image does not support externally linked r:link pictures in V1.",
            )
            .with_location(self.location(Some(target), picture.link_rel_id)));
        }
        if picture.embed_rel_id.is_none() {
            return Err(Error::new(
                ErrorCode::UnsupportedEdit,
                "replace_image target picture has no r:embed media relationship.",
            )
            .with_location(self.location(Some(target), None)));
        }

        Ok(())
    }

    pub fn apply(
        &self,
        package: &mut Package,
        target: &ResolvedElement,
        media_inputs: &MediaInputs,
    ) -> Result<PatchEffects> {
        self.validate_target(target)?;
        let media = media_inputs
            .resolve(&self.media_ref)
            .map_err(|error| error.with_location(self.location(Some(target), None)))?;
        if media.content_type != self.content_type {
            return Err(Error::new(
                ErrorCode::UnsupportedMediaType,
                format!(
                    "replace_image content_type `{}` does not match bound media_ref `{}` content type `{}`.",
                    self.content_type, self.media_ref, media.content_type
                ),
            )
            .with_location(self.location(Some(target), None)));
        }

        let picture = self.inspect_picture(package, target)?;
        if let Some(link_rel_id) = picture.link_rel_id {
            return Err(Error::new(
                ErrorCode::UnsupportedEdit,
                "replace_image does not support externally linked r:link pictures in V1.",
            )
            .with_location(self.location(Some(target), Some(link_rel_id))));
        }
        let embed_rel_id = picture.embed_rel_id.ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                "replace_image target picture has no r:embed media relationship.",
            )
            .with_location(self.location(Some(target), None))
        })?;

        let old_media_part = relationship_target_part(package, &target.part, &embed_rel_id)
            .map_err(|error| {
                error.with_location(self.location(Some(target), Some(embed_rel_id.clone())))
            })?;

        let extension = extension_for_content_type(&self.content_type).ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedMediaType,
                format!(
                    "replace_image content_type `{}` is not supported in V1.",
                    self.content_type
                ),
            )
            .with_location(self.location(Some(target), None))
        })?;
        let new_media_part = next_media_part_name(package.parts(), extension)
            .map_err(|error| error.with_location(self.location(Some(target), None)))?;
        package
            .insert_zip_entry(new_media_part.zip_entry_name(), media.bytes)
            .map_err(|error| error.with_location(self.location(Some(target), None)))?;
        let content_type_changed =
            ensure_content_type(package, &new_media_part, extension, &self.content_type);

        let target_value = relative_target(&target.part, &new_media_part);
        retarget_relationship(
            package,
            &target.part,
            &embed_rel_id,
            &target_value,
            &new_media_part,
        )
        .map_err(|error| {
            error.with_location(self.location(Some(target), Some(embed_rel_id.clone())))
        })?;

        let rels_part = rels_part_name_for(&target.part)?;
        package.mark_dirty(rels_part.clone());
        package.mark_dirty(new_media_part.clone());
        let cleanup_content_type_changed =
            cleanup_unreferenced_media_part(package, &old_media_part);
        if content_type_changed {
            package.mark_dirty(content_types_part()?);
        }
        if cleanup_content_type_changed {
            package.mark_dirty(content_types_part()?);
        }

        let mut changed_parts = vec![
            rels_part.zip_entry_name().to_owned(),
            new_media_part.zip_entry_name().to_owned(),
        ];
        if content_type_changed || cleanup_content_type_changed {
            changed_parts.push("[Content_Types].xml".to_owned());
        }

        Ok(PatchEffects {
            changed_parts,
            target: Some(OperationTarget {
                slide_id: target.slide_id.clone(),
                element_id: target.element_id.clone(),
                part: target.part.zip_entry_name().to_owned(),
            }),
            created_element_ids: Vec::new(),
            warnings: Vec::new(),
        })
    }

    fn validate_target(&self, target: &ResolvedElement) -> Result<()> {
        if target.element_id != self.element_id {
            return Err(Error::new(
                ErrorCode::SelectorGuardFailed,
                "Resolved element does not match replace_image element_id.",
            )
            .with_location(self.location(Some(target), None)));
        }
        if target.kind != ElementKind::Picture {
            return Err(Error::new(
                ErrorCode::UnsupportedEdit,
                "replace_image target must be a picture element.",
            )
            .with_location(self.location(Some(target), None)));
        }
        Ok(())
    }

    fn inspect_picture(&self, package: &Package, target: &ResolvedElement) -> Result<PictureBlip> {
        let part = package.parts().get(&target.part).ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                format!("Target slide part {} was not found.", target.part),
            )
            .with_location(self.location(Some(target), None))
        })?;
        let document = parse_document(part.bytes()).map_err(|source| {
            Error::with_source(
                source.code(),
                format!("Could not parse target slide part {}.", target.part),
                source,
            )
            .with_location(self.location(Some(target), None))
        })?;
        let picture = target_element(&document, target).ok_or_else(|| {
            Error::new(
                ErrorCode::SelectorNotFound,
                "Target picture path no longer resolves in the slide shape tree.",
            )
            .with_location(self.location(Some(target), None))
        })?;
        let blip = first_descendant(picture, "blip").ok_or_else(|| {
            Error::new(
                ErrorCode::UnsupportedEdit,
                "Target picture does not contain an a:blip element.",
            )
            .with_location(self.location(Some(target), None))
        })?;

        Ok(PictureBlip {
            embed_rel_id: attr(blip, "embed").map(str::to_owned),
            link_rel_id: attr(blip, "link").map(str::to_owned),
        })
    }

    fn location(
        &self,
        target: Option<&ResolvedElement>,
        relationship_id: Option<String>,
    ) -> ErrorLocation {
        ErrorLocation {
            part: target.map(|target| target.part.zip_entry_name().to_owned()),
            relationship_id,
            slide_id: target.map(|target| target.slide_id.clone()),
            element_id: Some(
                target
                    .map(|target| target.element_id.clone())
                    .unwrap_or_else(|| self.element_id.clone()),
            ),
            operation_id: Some(self.operation_id.clone()),
            operation: Some("replace_image".to_owned()),
            ..ErrorLocation::default()
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PictureBlip {
    embed_rel_id: Option<String>,
    link_rel_id: Option<String>,
}

fn relationship_target_part(
    package: &Package,
    source_part: &PartName,
    rel_id: &str,
) -> Result<PartName> {
    let rels = package
        .relationships()
        .set_for(source_part)
        .ok_or_else(|| {
            Error::unsupported_package(format!("Relationship set for {source_part} is missing."))
        })?;
    let relationship = rels.get(rel_id).ok_or_else(|| {
        Error::unsupported_package(format!(
            "Picture relationship {rel_id} is missing from {source_part}."
        ))
    })?;
    if relationship.target_mode != TargetMode::Internal || relationship.rel_type != IMAGE_REL_TYPE {
        return Err(Error::new(
            ErrorCode::UnsupportedEdit,
            format!("Picture relationship {rel_id} is not an internal image relationship."),
        ));
    }
    relationship.resolved_target.clone().map_or_else(
        || resolve_internal_target(&relationship.source, &relationship.target),
        Ok,
    )
}

fn retarget_relationship(
    package: &mut Package,
    source_part: &PartName,
    rel_id: &str,
    target_value: &str,
    resolved_target: &PartName,
) -> Result<()> {
    let relationship = {
        let rels = package
            .relationships()
            .set_for(source_part)
            .ok_or_else(|| {
                Error::unsupported_package(format!(
                    "Relationship set for {source_part} is missing."
                ))
            })?;
        rels.get(rel_id).cloned().ok_or_else(|| {
            Error::unsupported_package(format!(
                "Picture relationship {rel_id} is missing from {source_part}."
            ))
        })?
    };
    let mut replacement = relationship;
    replacement.target = target_value.to_owned();
    replacement.target_mode = TargetMode::Internal;
    replacement.mode = TargetMode::Internal;
    replacement.resolved_target = Some(resolved_target.clone());
    package.relationships_mut().replace(replacement)?;

    let rels_part = rels_part_name_for(source_part)?;
    if let Some(part) = package.parts_mut().get_mut(&rels_part) {
        let mut document = parse_document(part.bytes())?;
        rewrite_relationship_target(&mut document, rel_id, target_value)?;
        *part.bytes_mut() = write_document(
            &document,
            &WriteOptions {
                mode: WriteMode::Preserve,
            },
        )?;
    }

    Ok(())
}

fn rewrite_relationship_target(
    document: &mut XmlDocument,
    rel_id: &str,
    target_value: &str,
) -> Result<()> {
    let root = root_element_mut(document).ok_or_else(|| {
        Error::malformed_xml("Relationship part does not contain a root element.")
    })?;
    let relationship = root
        .children
        .iter_mut()
        .filter_map(node_element_mut)
        .find(|element| {
            element.name.local_name == "Relationship"
                && attr(element, "Id").is_some_and(|id| id == rel_id)
        })
        .ok_or_else(|| {
            Error::unsupported_package(format!(
                "Relationship {rel_id} was not found in relationship part XML."
            ))
        })?;
    set_attribute(relationship, "Target", target_value);
    remove_attribute(relationship, "TargetMode");
    Ok(())
}

fn target_element<'a>(
    document: &'a XmlDocument,
    target: &ResolvedElement,
) -> Option<&'a XmlElement> {
    let root = document.root_element()?;
    let sp_tree = first_descendant(root, "spTree")?;
    element_at_path(sp_tree, &target.sp_tree_path)
}

fn element_at_path<'a>(sp_tree: &'a XmlElement, path: &[u32]) -> Option<&'a XmlElement> {
    let mut current = sp_tree;
    for component in path {
        let index = usize::try_from(component.checked_sub(1)?).ok()?;
        current = current
            .children
            .iter()
            .filter_map(XmlNode::as_element)
            .filter(|element| is_real_shape_tree_child(element))
            .nth(index)?;
    }
    Some(current)
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

fn node_element_mut(node: &mut XmlNode) -> Option<&mut XmlElement> {
    match node {
        XmlNode::Element(element) => Some(element),
        XmlNode::Text(_)
        | XmlNode::CData(_)
        | XmlNode::Comment(_)
        | XmlNode::ProcessingInstruction(_)
        | XmlNode::DocType(_)
        | XmlNode::GeneralRef(_) => None,
    }
}

fn root_element_mut(document: &mut XmlDocument) -> Option<&mut XmlElement> {
    document.nodes.iter_mut().find_map(node_element_mut)
}

fn attr<'a>(element: &'a XmlElement, local_name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local_name == local_name)
        .map(|attribute| attribute.value.as_str())
}

fn set_attribute(element: &mut XmlElement, local_name: &str, value: &str) {
    if let Some(attribute) = element
        .attributes
        .iter_mut()
        .find(|attribute| attribute.name.local_name == local_name)
    {
        attribute.value = value.to_owned();
    } else {
        element.attributes.push(XmlAttribute {
            name: pptx_compose_core::xml::document::QualifiedName::from_raw(local_name),
            value: value.to_owned(),
            namespace_declaration: false,
        });
    }
}

fn remove_attribute(element: &mut XmlElement, local_name: &str) {
    element
        .attributes
        .retain(|attribute| attribute.name.local_name != local_name);
}

fn cleanup_unreferenced_media_part(package: &mut Package, media_part: &PartName) -> bool {
    if image_relationship_ref_count(package, media_part) != 0 {
        return false;
    }

    if package.remove_part(media_part).is_none() {
        return false;
    }

    let mut changed = package.content_types_mut().remove_override(media_part);
    if let Some(extension) = part_extension(media_part)
        && content_type_default_is_unused(package, extension)
    {
        changed |= package.content_types_mut().remove_default(extension);
    }
    changed
}

fn image_relationship_ref_count(package: &Package, media_part: &PartName) -> usize {
    package
        .relationships()
        .iter()
        .filter(|relationship| {
            relationship.target_mode == TargetMode::Internal
                && relationship.rel_type == IMAGE_REL_TYPE
                && relationship
                    .resolved_target
                    .as_ref()
                    .is_some_and(|target| target == media_part)
        })
        .count()
}

fn content_type_default_is_unused(package: &Package, extension: &str) -> bool {
    package.parts().iter().all(|part| {
        if part_extension(part.name()) != Some(extension) {
            return true;
        }
        package.content_types().override_for(part.name()).is_some()
    })
}

fn part_extension(part_name: &PartName) -> Option<&str> {
    let file_name = part_name.as_str().rsplit('/').next()?;
    let (_, extension) = file_name.rsplit_once('.')?;
    if extension.is_empty() {
        None
    } else {
        Some(extension)
    }
}

fn extension_for_content_type(content_type: &str) -> Option<&'static str> {
    match content_type {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        _ => None,
    }
}

fn relative_target(source_part: &PartName, target_part: &PartName) -> String {
    let source_segments = source_part
        .as_str()
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();
    let target_segments = target_part
        .as_str()
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();
    let source_dir = &source_segments[..source_segments.len().saturating_sub(1)];
    let mut common = 0_usize;
    while common < source_dir.len()
        && common < target_segments.len()
        && source_dir[common] == target_segments[common]
    {
        common += 1;
    }

    let mut relative = Vec::new();
    relative.extend(std::iter::repeat_n(
        "..",
        source_dir.len().saturating_sub(common),
    ));
    relative.extend(target_segments[common..].iter().copied());
    relative.join("/")
}

fn rels_part_name_for(part_name: &PartName) -> Result<PartName> {
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

fn content_types_part() -> Result<PartName> {
    PartName::from_zip_entry("[Content_Types].xml")
}

#[cfg(test)]
pub mod retargets_not_shared {
    use std::collections::HashMap;

    use pptx_compose_core::{
        error::ErrorCode,
        opc::{
            package::Package,
            part_name::PartName,
            relationships::{Relationship, RelationshipSource, resolve_internal_target},
        },
        pptx::{ids::ElementKind, media::IMAGE_REL_TYPE},
    };

    use super::*;
    use crate::media_inputs::{MediaBinding, MediaInputs, MediaSource};

    #[test]
    fn shared_media_retargets_only_selected_picture_and_rejects_link() {
        let slide1 = part("ppt/slides/slide1.xml");
        let slide2 = part("ppt/slides/slide2.xml");
        let mut package = Package::new();
        package
            .insert_zip_entry(slide1.zip_entry_name(), slide_xml("rId5").into_bytes())
            .expect("slide1 inserted");
        package
            .insert_zip_entry(slide2.zip_entry_name(), slide_xml("rId7").into_bytes())
            .expect("slide2 inserted");
        package
            .insert_zip_entry(
                "ppt/slides/_rels/slide1.xml.rels",
                rels_xml("../media/image1.png", "rId5").into_bytes(),
            )
            .expect("slide1 rels inserted");
        package
            .insert_zip_entry(
                "ppt/slides/_rels/slide2.xml.rels",
                rels_xml("../media/image1.png", "rId7").into_bytes(),
            )
            .expect("slide2 rels inserted");
        package
            .insert_zip_entry("ppt/media/image1.png", one_by_one_png())
            .expect("shared media inserted");
        package
            .content_types_mut()
            .insert_default("png", "image/png");
        package
            .content_types_mut()
            .insert_default("jpg", "image/jpeg");
        package.push_relationship(Relationship::internal(
            RelationshipSource::Part(slide1.clone()),
            "rId5",
            IMAGE_REL_TYPE,
            "../media/image1.png",
        ));
        package.push_relationship(Relationship::internal(
            RelationshipSource::Part(slide2.clone()),
            "rId7",
            IMAGE_REL_TYPE,
            "../media/image1.png",
        ));

        let operation = ReplaceImage {
            operation_id: "op-1".to_owned(),
            element_id: "slide-1:pic-1".to_owned(),
            media_ref: "replacement".to_owned(),
            content_type: "image/png".to_owned(),
            allow_shared_mutation: true,
        };
        let effects = operation
            .apply(&mut package, &target(&slide1), &media_inputs())
            .expect("shared image replacement retargets");

        assert_eq!(
            effects.changed_parts,
            vec!["ppt/slides/_rels/slide1.xml.rels", "ppt/media/image2.png"]
        );
        assert!(
            !package
                .dirty_parts()
                .contains(&content_types_part().expect("valid part"))
        );
        let slide1_target = relationship_target(&package, &slide1, "rId5");
        let slide2_target = relationship_target(&package, &slide2, "rId7");
        assert_eq!(slide1_target.as_str(), "/ppt/media/image2.png");
        assert_eq!(slide2_target.as_str(), "/ppt/media/image1.png");
        let rels_part = package
            .parts()
            .get(&part("ppt/slides/_rels/slide1.xml.rels"))
            .expect("slide1 rels exists");
        assert!(
            std::str::from_utf8(rels_part.bytes())
                .expect("rels utf8")
                .contains("../media/image2.png")
        );

        let link = ReplaceImage {
            operation_id: "op-link".to_owned(),
            element_id: "slide-1:pic-1".to_owned(),
            media_ref: "replacement".to_owned(),
            content_type: "image/png".to_owned(),
            allow_shared_mutation: false,
        };
        let mut linked_package = package.clone();
        *linked_package
            .parts_mut()
            .get_mut(&slide1)
            .expect("slide exists")
            .bytes_mut() = link_slide_xml("rId9").into_bytes();
        let error = link
            .validate(&linked_package, &target(&slide1), &media_inputs())
            .expect_err("r:link image replacement is unsupported");
        assert_eq!(error.code(), ErrorCode::UnsupportedEdit);
    }

    #[test]
    fn retarget_adds_content_type_default_when_needed() {
        let slide = part("ppt/slides/slide1.xml");
        let mut package = Package::new();
        package
            .insert_zip_entry(slide.zip_entry_name(), slide_xml("rId5").into_bytes())
            .expect("slide inserted");
        package
            .insert_zip_entry(
                "ppt/slides/_rels/slide1.xml.rels",
                rels_xml("../media/image1.png", "rId5").into_bytes(),
            )
            .expect("slide rels inserted");
        package
            .insert_zip_entry("ppt/media/image1.png", one_by_one_png())
            .expect("old media inserted");
        package.push_relationship(Relationship::internal(
            RelationshipSource::Part(slide.clone()),
            "rId5",
            IMAGE_REL_TYPE,
            "../media/image1.png",
        ));

        let operation = ReplaceImage {
            operation_id: "op-1".to_owned(),
            element_id: "slide-1:pic-1".to_owned(),
            media_ref: "replacement".to_owned(),
            content_type: "image/png".to_owned(),
            allow_shared_mutation: false,
        };
        let effects = operation
            .apply(&mut package, &target(&slide), &media_inputs())
            .expect("image replacement retargets");

        assert_eq!(
            effects.changed_parts,
            vec![
                "ppt/slides/_rels/slide1.xml.rels",
                "ppt/media/image2.png",
                "[Content_Types].xml"
            ]
        );
        let new_media = part("ppt/media/image2.png");
        assert_eq!(
            package.content_types().resolve(&new_media),
            Some("image/png")
        );
        assert!(package.parts().get(&part("ppt/media/image1.png")).is_none());
        assert!(
            package
                .dirty_parts()
                .contains(&content_types_part().expect("valid part"))
        );
    }

    #[test]
    fn unshared_retarget_removes_old_media_part_and_stale_override() {
        let slide = part("ppt/slides/slide1.xml");
        let old_media = part("ppt/media/image1.png");
        let mut package = Package::new();
        package
            .insert_zip_entry(slide.zip_entry_name(), slide_xml("rId5").into_bytes())
            .expect("slide inserted");
        package
            .insert_zip_entry(
                "ppt/slides/_rels/slide1.xml.rels",
                rels_xml("../media/image1.png", "rId5").into_bytes(),
            )
            .expect("slide rels inserted");
        package
            .insert_zip_entry(old_media.zip_entry_name(), one_by_one_png())
            .expect("old media inserted");
        package
            .content_types_mut()
            .insert_override(old_media.clone(), "image/png");
        package.push_relationship(Relationship::internal(
            RelationshipSource::Part(slide.clone()),
            "rId5",
            IMAGE_REL_TYPE,
            "../media/image1.png",
        ));

        let operation = ReplaceImage {
            operation_id: "op-1".to_owned(),
            element_id: "slide-1:pic-1".to_owned(),
            media_ref: "replacement".to_owned(),
            content_type: "image/jpeg".to_owned(),
            allow_shared_mutation: false,
        };
        let effects = operation
            .apply(&mut package, &target(&slide), &jpeg_media_inputs())
            .expect("image replacement retargets");

        let new_media = part("ppt/media/image1.jpg");
        assert_eq!(
            effects.changed_parts,
            vec![
                "ppt/slides/_rels/slide1.xml.rels",
                "ppt/media/image1.jpg",
                "[Content_Types].xml"
            ]
        );
        assert!(package.parts().get(&old_media).is_none());
        assert_eq!(package.content_types().override_for(&old_media), None);
        assert_eq!(package.content_types().default_for_ext("png"), None);
        assert_eq!(
            package.content_types().resolve(&new_media),
            Some("image/jpeg")
        );
        assert_eq!(relationship_target(&package, &slide, "rId5"), new_media);
    }

    fn relationship_target(package: &Package, source: &PartName, rel_id: &str) -> PartName {
        package
            .relationships()
            .set_for(source)
            .and_then(|set| set.get(rel_id))
            .and_then(|relationship| {
                relationship.resolved_target.clone().or_else(|| {
                    resolve_internal_target(&relationship.source, &relationship.target).ok()
                })
            })
            .expect("relationship resolves")
    }

    fn target(part: &PartName) -> ResolvedElement {
        ResolvedElement {
            slide_id: "slide-1".to_owned(),
            element_id: "slide-1:pic-1".to_owned(),
            kind: ElementKind::Picture,
            part: part.clone(),
            sp_tree_path: vec![1],
            group_path: Vec::new(),
            cnvpr_id: Some(7),
            text_hash: None,
            fingerprint: "sha256:test".to_owned(),
        }
    }

    fn media_inputs() -> MediaInputs {
        let mut bindings = HashMap::new();
        bindings.insert(
            "replacement".to_owned(),
            MediaBinding {
                content_type: "image/png".to_owned(),
                declared_sha256: None,
                declared_byte_length: None,
                source: MediaSource::Bytes(two_by_two_png()),
            },
        );
        MediaInputs::new(bindings)
    }

    fn jpeg_media_inputs() -> MediaInputs {
        let mut bindings = HashMap::new();
        bindings.insert(
            "replacement".to_owned(),
            MediaBinding {
                content_type: "image/jpeg".to_owned(),
                declared_sha256: None,
                declared_byte_length: None,
                source: MediaSource::Bytes(jpeg_bytes()),
            },
        );
        MediaInputs::new(bindings)
    }

    fn part(name: &str) -> PartName {
        PartName::from_zip_entry(name).expect("valid part name")
    }

    fn slide_xml(rel_id: &str) -> String {
        format!(
            r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:pic><p:nvPicPr><p:cNvPr id="7" name="Picture 1"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="{rel_id}"/></p:blipFill><p:spPr/></p:pic></p:spTree></p:cSld></p:sld>"#
        )
    }

    fn link_slide_xml(rel_id: &str) -> String {
        format!(
            r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:pic><p:nvPicPr><p:cNvPr id="7" name="Picture 1"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:link="{rel_id}"/></p:blipFill><p:spPr/></p:pic></p:spTree></p:cSld></p:sld>"#
        )
    }

    fn rels_xml(target: &str, rel_id: &str) -> String {
        format!(
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="{rel_id}" Type="{IMAGE_REL_TYPE}" Target="{target}"/></Relationships>"#
        )
    }

    fn one_by_one_png() -> Vec<u8> {
        vec![
            0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0x00, 0x00, 0x00, 0x0d, b'I', b'H',
            b'D', b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1f, 0x15, 0xc4, 0x89,
        ]
    }

    fn two_by_two_png() -> Vec<u8> {
        vec![
            0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0x00, 0x00, 0x00, 0x0d, b'I', b'H',
            b'D', b'R', 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x72, 0xb6, 0x0d, 0x24,
        ]
    }

    fn jpeg_bytes() -> Vec<u8> {
        vec![0xff, 0xd8, 0xff, 0xe0, b'J', b'F', b'I', b'F', 0x00]
    }
}
