#[path = "support/fixtures.rs"]
mod fixtures;

const REQUIRED_FIXTURE_FAMILIES: &[&str] = &[
    "legacy/",
    "minimal/",
    "powerpoint/",
    "libreoffice/",
    "google-slides/",
    "media/",
    "charts/",
    "embedded/",
    "malformed/",
];

const MANIFEST_FIXTURE_FAMILIES: &[&str] = &[
    "legacy/",
    "minimal/",
    "powerpoint/",
    "libreoffice/",
    "google-slides/",
    "media/",
    "charts/",
    "embedded/",
    "malformed/",
    "real-world/",
];

mod manifest {
    use std::path::Path;

    use super::REQUIRED_FIXTURE_FAMILIES;
    use super::fixtures::{fixture_path, load_manifest};

    #[test]
    fn every_entry_file_exists() {
        let manifest = load_manifest();

        for entry in manifest.entries {
            assert!(
                fixture_path(&entry.path).exists(),
                "fixture path does not exist: {}",
                entry.path
            );
            assert!(
                !entry.consuming_test.trim().is_empty(),
                "fixture entry has an empty consuming_test: {}",
                entry.path
            );
        }
    }

    #[test]
    fn every_required_family_pptx_has_manifest_entry() {
        let manifest = load_manifest();

        for family in REQUIRED_FIXTURE_FAMILIES {
            let family_path = fixture_path(family);
            let entries = std::fs::read_dir(&family_path).unwrap_or_else(|error| {
                panic!(
                    "could not read fixture family {}: {error}",
                    family_path.display()
                )
            });

            for entry in entries {
                let entry = entry.unwrap_or_else(|error| {
                    panic!(
                        "could not read fixture family entry {}: {error}",
                        family_path.display()
                    )
                });
                let path = entry.path();
                if !is_pptx(&path) {
                    continue;
                }

                let rel_path = format!(
                    "{}{}",
                    family,
                    path.file_name()
                        .expect("fixture file has a file name")
                        .to_string_lossy()
                );
                assert!(
                    manifest.contains_path(&rel_path),
                    "fixture manifest must include PPTX fixture `{rel_path}`"
                );
            }
        }
    }

    fn is_pptx(path: &Path) -> bool {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pptx"))
    }
}

mod corpus {
    use pptx_compose_core::{
        validation::{FindingCode, Severity, ValidationMode, ValidationStatus, validate_package},
        xml::parser::parse_document,
        zip::reader::from_bytes,
    };

    use super::fixtures::{SourceApp, fixture_path, load_manifest};

    #[test]
    fn manifest_covers_required_fixture_families() {
        let manifest = load_manifest();

        for source_app in [
            SourceApp::PowerPoint,
            SourceApp::LibreOffice,
            SourceApp::GoogleSlides,
            SourceApp::Legacy,
            SourceApp::Synthetic,
        ] {
            assert!(
                manifest.has_source_app(source_app),
                "fixture manifest must include source_app `{source_app:?}`"
            );
        }

        for required_prefix in super::MANIFEST_FIXTURE_FAMILIES {
            assert!(
                manifest.has_path_prefix(required_prefix),
                "fixture manifest must include fixture family `{required_prefix}`"
            );
        }

        for required_feature in [
            "legacy-smoke",
            "minimal-package",
            "source-family",
            "media",
            "chart",
            "embedding",
            "malformed",
            "real-world",
        ] {
            assert!(
                manifest.has_feature(required_feature),
                "fixture manifest must include feature `{required_feature}`"
            );
        }

        for source_app in [
            SourceApp::PowerPoint,
            SourceApp::LibreOffice,
            SourceApp::GoogleSlides,
        ] {
            assert!(
                manifest
                    .entries_with_feature("source-family")
                    .any(|entry| entry.source_app == source_app),
                "fixture manifest must include a source-family fixture for `{source_app:?}`"
            );
        }

        assert!(
            manifest
                .entries_with_feature("real-world")
                .any(|entry| entry.has_invariant("roundtrip")),
            "fixture manifest must include a real-world roundtrip fixture"
        );
    }

    #[test]
    fn malformed_fixture_is_rejected_when_slide_xml_is_parsed() {
        let manifest = load_manifest();
        let malformed = manifest
            .entries
            .iter()
            .find(|entry| entry.has_invariant("malformed"))
            .expect("manifest includes a malformed fixture");
        let package =
            std::fs::read(fixture_path(&malformed.path)).expect("malformed fixture reads");
        let entries = from_bytes(&package).expect("malformed fixture is still a ZIP package");
        let slide = entries
            .iter()
            .find(|entry| {
                let name = entry.name.zip_entry_name();
                name.starts_with("ppt/slides/") && name.ends_with(".xml")
            })
            .expect("malformed fixture contains a slide XML part");

        let error = parse_document(&slide.bytes).expect_err("malformed slide XML is rejected");
        super::fixtures::assert_expected_warnings(
            &malformed.path,
            &malformed.expected_warnings,
            [error.code().as_str()],
        );

        let package = super::fixtures::package_from_entries(&entries)
            .expect("malformed fixture entries hydrate as an OPC package");

        let outcome = validate_package(&package, ValidationMode::NoEdit);

        assert_eq!(outcome.status, ValidationStatus::Invalid);
        let malformed_xml = outcome
            .findings
            .iter()
            .find(|finding| finding.code == FindingCode::MalformedXml)
            .expect("validate reports malformed_xml for the malformed fixture");
        assert_eq!(malformed_xml.severity, Severity::Fatal);
        assert!(
            malformed_xml.blocking,
            "fatal malformed_xml blocks no-edit validation"
        );
    }
}

mod persistence {
    use std::io::Cursor;

    use pptx_compose_core::zip::{
        reader::from_bytes,
        writer::{WriteEntry, write_vec},
    };
    use zip::ZipArchive;

    use super::fixtures::{fixture_path, load_manifest};

    #[test]
    fn manifest_pptx_fixtures_no_edit_round_trip_as_clean_entries() {
        let manifest = load_manifest();

        for entry in manifest.entries {
            if !entry.path.ends_with(".pptx") {
                continue;
            }
            if entry.has_invariant("malformed") {
                continue;
            }

            let path = fixture_path(&entry.path);
            let package = std::fs::read(&path)
                .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
            let entries = from_bytes(&package)
                .unwrap_or_else(|error| panic!("could not parse {}: {error}", path.display()));
            let write_entries: Vec<_> = entries.iter().map(WriteEntry::Clean).collect();

            let written = write_vec(&package, &write_entries)
                .unwrap_or_else(|error| panic!("could not persist {}: {error}", path.display()));
            let reparsed = from_bytes(&written)
                .unwrap_or_else(|error| panic!("could not reparse {}: {error}", path.display()));

            assert_eq!(
                reparsed.len(),
                entries.len(),
                "round-trip changed ZIP entry count for {}",
                entry.path
            );
            assert_clean_entries_preserved(&entry.path, &package, &written);
        }
    }

    fn assert_clean_entries_preserved(fixture: &str, original: &[u8], written: &[u8]) {
        let mut original_archive = ZipArchive::new(Cursor::new(original))
            .unwrap_or_else(|error| panic!("could not reopen original {fixture}: {error}"));
        let mut written_archive = ZipArchive::new(Cursor::new(written))
            .unwrap_or_else(|error| panic!("could not reopen written {fixture}: {error}"));

        assert_eq!(
            written_archive.len(),
            original_archive.len(),
            "round-trip changed archive length for {fixture}"
        );

        for index in 0..original_archive.len() {
            let original_entry = original_archive
                .by_index(index)
                .unwrap_or_else(|error| panic!("could not read original {fixture}: {error}"));
            let written_entry = written_archive
                .by_index(index)
                .unwrap_or_else(|error| panic!("could not read written {fixture}: {error}"));

            assert_eq!(
                written_entry.name(),
                original_entry.name(),
                "round-trip changed entry name at index {index} for {fixture}"
            );
            assert_eq!(
                written_entry.crc32(),
                original_entry.crc32(),
                "round-trip changed CRC for {} in {fixture}",
                original_entry.name()
            );
            assert_eq!(
                compressed_bytes(written, index),
                compressed_bytes(original, index),
                "round-trip changed clean compressed bytes for {} in {fixture}",
                original_entry.name()
            );
        }
    }

    fn compressed_bytes(package: &[u8], index: usize) -> &[u8] {
        let mut archive = ZipArchive::new(Cursor::new(package)).expect("package opens");
        let file = archive.by_index(index).expect("entry exists");
        let start = file.data_start().expect("entry data start exists") as usize;
        let end = start + file.compressed_size() as usize;
        &package[start..end]
    }
}
