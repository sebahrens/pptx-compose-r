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

const KNOWN_CONSUMING_TESTS: &[&str] = &[
    "fixture_manifest::corpus::malformed_fixture_is_rejected_when_slide_xml_is_parsed",
    "fixture_manifest::corpus::manifest_covers_required_fixture_families",
    "fixture_manifest::persistence::manifest_pptx_fixtures_no_edit_round_trip_as_clean_entries",
    "negative_cases::invalid_inputs_fail_before_mutation",
    "roundtrip_golden::edits::add_image_runs_against_corpus_media_fixture",
    "roundtrip_golden::roundtrip::no_edit_byte_identity",
];

mod manifest {
    use std::path::Path;

    use super::fixtures::{fixture_path, load_manifest};
    use super::{KNOWN_CONSUMING_TESTS, REQUIRED_FIXTURE_FAMILIES};

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
            assert!(
                KNOWN_CONSUMING_TESTS.contains(&entry.consuming_test.as_str()),
                "fixture entry references an unknown consuming_test `{}`: {}",
                entry.consuming_test,
                entry.path
            );
        }
    }

    #[test]
    fn every_v1_pptx_fixture_has_manifest_entry() {
        let manifest = load_manifest();

        for rel_path in pptx_fixture_paths() {
            assert!(
                manifest.contains_path(&rel_path),
                "fixture manifest must include PPTX fixture `{rel_path}`"
            );
        }
    }

    #[test]
    fn required_families_are_present_on_disk() {
        for family in REQUIRED_FIXTURE_FAMILIES {
            let family_path = fixture_path(family);
            assert!(
                family_path.is_dir(),
                "required fixture family must exist: {}",
                family_path.display()
            );
        }
    }

    fn pptx_fixture_paths() -> Vec<String> {
        let root = fixture_path("");
        let mut paths = Vec::new();
        collect_pptx_fixture_paths(&root, &root, &mut paths);
        paths.sort();
        paths
    }

    fn collect_pptx_fixture_paths(root: &Path, directory: &Path, paths: &mut Vec<String>) {
        let mut entries = std::fs::read_dir(directory)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", directory.display()))
            .map(|entry| {
                entry.unwrap_or_else(|error| {
                    panic!("could not read entry in {}: {error}", directory.display())
                })
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.path());

        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                collect_pptx_fixture_paths(root, &path, paths);
            } else if is_pptx(&path) {
                let rel_path = path
                    .strip_prefix(root)
                    .expect("fixture path is under fixture root")
                    .to_string_lossy()
                    .replace('\\', "/");
                paths.push(rel_path);
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
        validation::{FindingCode, ValidationMode, ValidationStatus, validate_package},
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
                !manifest.entries_for_source_app(source_app).is_empty(),
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
            "localized",
            "localized-stale-evidence",
        ] {
            assert!(
                manifest.has_feature(required_feature),
                "fixture manifest must include feature `{required_feature}`"
            );
        }

        for expected_warning in [
            "external_relationship_not_checked",
            "malformed_xml",
            "unsafe_path",
        ] {
            assert!(
                manifest.has_warning(expected_warning),
                "fixture manifest must include expected warning `{expected_warning}`"
            );
        }

        for required_invariant in [
            "contains-presentation-part",
            "roundtrip",
            "edit-add-image",
            "malformed",
            "unsafe-path",
        ] {
            assert!(
                manifest.has_invariant(required_invariant),
                "fixture manifest must include invariant `{required_invariant}`"
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
                    .into_iter()
                    .any(|entry| entry.source_app == source_app),
                "fixture manifest must include a source-family fixture for `{source_app:?}`"
            );
        }

        assert!(
            manifest
                .entries_with_feature("real-world")
                .into_iter()
                .any(|entry| entry.has_invariant("roundtrip")),
            "fixture manifest must include a real-world roundtrip fixture"
        );

        assert!(
            !manifest
                .entries_consumed_by(
                    "fixture_manifest::persistence::manifest_pptx_fixtures_no_edit_round_trip_as_clean_entries"
                )
                .is_empty(),
            "fixture manifest must record entries consumed by persistence tests"
        );
    }

    #[test]
    fn localized_real_world_fixtures_are_marked_as_stale_translation_evidence() {
        let manifest = load_manifest();
        let localized = manifest.entries_with_feature("localized");

        assert!(
            !localized.is_empty(),
            "fixture manifest must include localized real-world fixtures"
        );

        for entry in localized {
            assert!(
                entry.has_feature("localized-stale-evidence"),
                "{}: localized fixtures must be relabeled until regenerated with guarded selectors across every V1-supported visible text class",
                entry.path
            );
            assert!(
                !entry.has_feature("localized-complete-evidence"),
                "{}: stale localized fixtures must not advertise complete V1 translation coverage",
                entry.path
            );
        }
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

        assert!(
            outcome
                .findings
                .iter()
                .all(|finding| finding.code != FindingCode::MalformedXml),
            "clean malformed XML parts are raw-copied and do not block no-edit validation"
        );
        assert_eq!(outcome.status, ValidationStatus::Valid);
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
