use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

use pptx_compose_core::{
    error::{Error, Result},
    opc::{
        package::Package,
        part::Part,
        part_name::PartName,
        relationships::{Relationship, RelationshipSource},
    },
    validation::{Severity, ValidationOutcome, ValidationStatus},
    xml::{document::XmlElement, parser::parse_document},
    zip::reader::RawEntry,
};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct FixtureManifest {
    pub entries: Vec<FixtureEntry>,
}

impl FixtureManifest {
    pub fn validate(&self) {
        assert!(
            !self.entries.is_empty(),
            "fixture manifest must include at least one entry"
        );

        let mut paths = BTreeSet::new();
        for entry in &self.entries {
            entry.validate();
            assert!(
                paths.insert(entry.path.as_str()),
                "fixture manifest contains duplicate path: {}",
                entry.path
            );
        }
    }

    #[allow(dead_code)]
    pub fn has_source_app(&self, source_app: SourceApp) -> bool {
        !self.entries_for_source_app(source_app).is_empty()
    }

    #[allow(dead_code)]
    pub fn has_path_prefix(&self, prefix: &str) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.path.starts_with(prefix))
    }

    #[allow(dead_code)]
    pub fn has_feature(&self, feature: &str) -> bool {
        !self.entries_with_feature(feature).is_empty()
    }

    #[allow(dead_code)]
    pub fn has_warning(&self, warning: &str) -> bool {
        !self.entries_with_warning(warning).is_empty()
    }

    #[allow(dead_code)]
    pub fn has_invariant(&self, invariant: &str) -> bool {
        !self.entries_with_invariant(invariant).is_empty()
    }

    #[allow(dead_code)]
    pub fn entries_for_source_app(&self, source_app: SourceApp) -> Vec<&FixtureEntry> {
        self.entries
            .iter()
            .filter(move |entry| entry.source_app == source_app)
            .collect()
    }

    #[allow(dead_code)]
    pub fn entries_with_feature(&self, feature: &str) -> Vec<&FixtureEntry> {
        self.entries
            .iter()
            .filter(move |entry| entry.has_feature(feature))
            .collect()
    }

    #[allow(dead_code)]
    pub fn entries_with_warning(&self, warning: &str) -> Vec<&FixtureEntry> {
        self.entries
            .iter()
            .filter(move |entry| entry.has_expected_warning(warning))
            .collect()
    }

    #[allow(dead_code)]
    pub fn entries_with_invariant(&self, invariant: &str) -> Vec<&FixtureEntry> {
        self.entries
            .iter()
            .filter(move |entry| entry.has_invariant(invariant))
            .collect()
    }

    #[allow(dead_code)]
    pub fn roundtrip_entries(&self) -> Vec<&FixtureEntry> {
        self.entries_with_invariant("roundtrip")
    }

    #[allow(dead_code)]
    pub fn entries_consumed_by(&self, consuming_test: &str) -> Vec<&FixtureEntry> {
        self.entries
            .iter()
            .filter(move |entry| entry.consuming_test == consuming_test)
            .collect()
    }

    #[allow(dead_code)]
    pub fn contains_path(&self, path: &str) -> bool {
        self.entries.iter().any(|entry| entry.path == path)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct FixtureEntry {
    pub path: String,
    pub source_app: SourceApp,
    pub features: Vec<String>,
    pub expected_warnings: Vec<String>,
    pub invariants: Vec<String>,
    pub consuming_test: String,
}

impl FixtureEntry {
    pub fn has_feature(&self, feature: &str) -> bool {
        self.features.iter().any(|item| item == feature)
    }

    pub fn has_invariant(&self, invariant: &str) -> bool {
        self.invariants.iter().any(|item| item == invariant)
    }

    pub fn has_expected_warning(&self, warning: &str) -> bool {
        self.expected_warnings.iter().any(|item| item == warning)
    }

    fn validate(&self) {
        assert!(
            !self.path.trim().is_empty(),
            "fixture manifest entry has an empty path"
        );
        assert!(
            is_safe_relative_path(&self.path),
            "fixture manifest entry path must be a safe relative path: {}",
            self.path
        );
        assert!(
            !self.features.is_empty(),
            "fixture manifest entry has no features: {}",
            self.path
        );
        assert!(
            !self.invariants.is_empty(),
            "fixture manifest entry has no invariants: {}",
            self.path
        );
        assert!(
            !self.consuming_test.trim().is_empty(),
            "fixture manifest entry has an empty consuming_test: {}",
            self.path
        );

        assert_non_empty_unique("features", &self.path, &self.features);
        assert_non_empty_unique("expected_warnings", &self.path, &self.expected_warnings);
        assert_non_empty_unique("invariants", &self.path, &self.invariants);
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "kebab-case")]
pub enum SourceApp {
    #[serde(rename = "powerpoint")]
    PowerPoint,
    #[serde(rename = "libreoffice")]
    LibreOffice,
    GoogleSlides,
    Legacy,
    Synthetic,
}

pub fn load_manifest() -> FixtureManifest {
    let manifest_path = fixture_path("manifest.toml");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap_or_else(|error| {
        panic!(
            "could not read fixture manifest at {}: {error}",
            manifest_path.display()
        )
    });
    let manifest: FixtureManifest = toml::from_str(&manifest).unwrap_or_else(|error| {
        panic!(
            "could not parse fixture manifest at {}: {error}",
            manifest_path.display()
        )
    });
    manifest.validate();
    manifest
}

pub fn fixture_path(rel: &str) -> PathBuf {
    fixtures_root().join(rel)
}

#[allow(dead_code)]
pub fn read_fixture_bytes(entry: &FixtureEntry) -> Vec<u8> {
    let path = fixture_path(&entry.path);
    std::fs::read(&path)
        .unwrap_or_else(|error| panic!("could not read fixture {}: {error}", path.display()))
}

pub fn assert_expected_warnings<I, S>(
    fixture: &str,
    expected_warnings: &[String],
    actual_warning_codes: I,
) where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let expected = expected_warnings
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let actual = actual_warning_codes
        .into_iter()
        .map(|code| code.as_ref().to_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        actual,
        expected
            .iter()
            .map(|warning| (*warning).to_owned())
            .collect::<BTreeSet<_>>(),
        "{fixture}: validation warnings did not match fixture manifest"
    );
}

#[allow(dead_code)]
pub fn assert_valid_with_expected_warnings(fixture: &FixtureEntry, validation: &ValidationOutcome) {
    assert_eq!(
        validation.status,
        ValidationStatus::Valid,
        "{}: no-edit package validation failed: {:#?}",
        fixture.path,
        validation.findings
    );
    assert_expected_warnings(
        &fixture.path,
        &fixture.expected_warnings,
        validation
            .findings
            .iter()
            .filter(|finding| finding.severity == Severity::Warning)
            .map(|finding| finding_code_to_string(finding.code)),
    );
}

#[allow(dead_code)]
pub fn assert_equal_part_sets(
    original_entries: &[RawEntry],
    written_entries: &[RawEntry],
    fixture: &str,
) {
    let original_names = part_names(original_entries);
    let written_names = part_names(written_entries);

    assert_eq!(
        written_names, original_names,
        "{fixture}: written package must contain exactly the original part set"
    );
}

#[allow(dead_code)]
pub fn assert_byte_identical_parts(
    original_entries: &[RawEntry],
    written_entries: &[RawEntry],
    fixture: &str,
) {
    let written_by_name = written_entries
        .iter()
        .map(|entry| (entry.name.clone(), entry))
        .collect::<BTreeMap<_, _>>();

    for original_entry in original_entries {
        let written_entry = written_by_name
            .get(&original_entry.name)
            .unwrap_or_else(|| {
                panic!("{fixture}: written package dropped {}", original_entry.name)
            });
        assert_eq!(
            written_entry.bytes, original_entry.bytes,
            "{fixture}: part {} changed in a no-edit round trip",
            original_entry.name
        );
    }
}

pub fn package_from_entries(entries: &[RawEntry]) -> Result<Package> {
    let mut package = Package::new();
    for entry in entries {
        package.insert_part(Part::from_zip_entry(
            entry.meta.original_name.clone(),
            entry.bytes.clone(),
        )?)?;
    }

    hydrate_content_types(&mut package)?;
    hydrate_relationships(&mut package)?;

    Ok(package)
}

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures")
}

fn part_names(entries: &[RawEntry]) -> BTreeSet<PartName> {
    entries.iter().map(|entry| entry.name.clone()).collect()
}

fn finding_code_to_string<T>(code: T) -> String
where
    T: serde::Serialize,
{
    serde_json::to_value(code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unserializable_finding_code".to_owned())
}

fn hydrate_content_types(package: &mut Package) -> Result<()> {
    let content_types_name = PartName::from_zip_entry("[Content_Types].xml")?;
    let raw = package
        .parts()
        .get(&content_types_name)
        .ok_or_else(|| Error::unsupported_package("Package is missing [Content_Types].xml."))?
        .bytes()
        .to_vec();
    let document = parse_document(&raw)?;
    let root = document
        .root_element()
        .ok_or_else(|| Error::unsupported_package("[Content_Types].xml has no root element."))?;

    if root.name.local_name != "Types" {
        return Err(Error::unsupported_package(
            "[Content_Types].xml root element is not Types.",
        ));
    }

    for child in root.children.iter().filter_map(|node| node.as_element()) {
        match child.name.local_name.as_str() {
            "Default" => {
                let extension = required_attr(child, "Extension")?;
                let content_type = required_attr(child, "ContentType")?;
                package
                    .content_types_mut()
                    .insert_default(extension, content_type);
            }
            "Override" => {
                let part_name = PartName::from_zip_entry(required_attr(child, "PartName")?)?;
                let content_type = required_attr(child, "ContentType")?;
                package
                    .content_types_mut()
                    .insert_override(part_name, content_type);
            }
            _ => {}
        }
    }

    Ok(())
}

fn hydrate_relationships(package: &mut Package) -> Result<()> {
    let rels_entries = package
        .parts()
        .iter()
        .filter(|part| part.name().as_str().ends_with(".rels"))
        .map(|part| (part.name().clone(), part.bytes().to_vec()))
        .collect::<Vec<_>>();

    for (rels_part_name, raw) in rels_entries {
        let source = relationship_source_for(&rels_part_name)?;
        let document = parse_document(&raw)?;
        let root = document
            .root_element()
            .ok_or_else(|| Error::unsupported_package(".rels part has no root element."))?;

        if root.name.local_name != "Relationships" {
            return Err(Error::unsupported_package(
                ".rels root element is not Relationships.",
            ));
        }

        for child in root.children.iter().filter_map(|node| node.as_element()) {
            if child.name.local_name != "Relationship" {
                continue;
            }

            let id = required_attr(child, "Id")?;
            let rel_type = required_attr(child, "Type")?;
            let target = required_attr(child, "Target")?;
            let relationship = if optional_attr(child, "TargetMode") == Some("External") {
                Relationship::external(source.clone(), id, rel_type, target)
            } else {
                Relationship::internal(source.clone(), id, rel_type, target)
            };
            package.push_relationship(relationship);
        }
    }

    Ok(())
}

fn relationship_source_for(rels_part_name: &PartName) -> Result<RelationshipSource> {
    let rels_path = rels_part_name.as_str();
    if rels_path == "/_rels/.rels" {
        return Ok(RelationshipSource::Package);
    }

    let Some((directory, file_name)) = rels_path.rsplit_once("/_rels/") else {
        return Err(Error::unsupported_package(format!(
            "Relationship part {rels_part_name} is not in an _rels directory."
        )));
    };
    let Some(source_file_name) = file_name.strip_suffix(".rels") else {
        return Err(Error::unsupported_package(format!(
            "Relationship part {rels_part_name} does not end with .rels."
        )));
    };

    PartName::from_zip_entry(format!("{directory}/{source_file_name}").as_str())
        .map(RelationshipSource::Part)
}

fn required_attr<'a>(element: &'a XmlElement, name: &str) -> Result<&'a str> {
    optional_attr(element, name).ok_or_else(|| {
        Error::unsupported_package(format!(
            "Element {} is missing required attribute {name}.",
            element.name.raw
        ))
    })
}

fn optional_attr<'a>(element: &'a XmlElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name.local_name == name)
        .map(|attribute| attribute.value.as_str())
}

fn assert_non_empty_unique(field: &str, path: &str, values: &[String]) {
    let mut seen = BTreeSet::new();
    for value in values {
        assert!(
            !value.trim().is_empty(),
            "fixture manifest entry has an empty {field} value: {path}"
        );
        assert!(
            seen.insert(value.as_str()),
            "fixture manifest entry has duplicate {field} value `{value}`: {path}"
        );
    }
}

fn is_safe_relative_path(path: &str) -> bool {
    let path = Path::new(path);
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}
