#[path = "support/fixtures.rs"]
mod fixtures;

mod manifest {
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
}

mod corpus {
    use std::collections::BTreeSet;

    use pptx_compose_core::{xml::parser::parse_document, zip::reader::from_bytes};

    use super::fixtures::{SourceApp, fixture_path, load_manifest};

    #[test]
    fn manifest_covers_required_fixture_families() {
        let manifest = load_manifest();

        let source_apps = manifest
            .entries
            .iter()
            .map(|entry| entry.source_app.clone())
            .collect::<BTreeSet<_>>();
        assert!(source_apps.contains(&SourceApp::PowerPoint));
        assert!(source_apps.contains(&SourceApp::LibreOffice));
        assert!(source_apps.contains(&SourceApp::GoogleSlides));

        for required_feature in ["media", "chart", "embedding", "malformed"] {
            assert!(
                manifest.entries.iter().any(|entry| entry
                    .features
                    .iter()
                    .any(|feature| feature == required_feature)),
                "fixture manifest must include feature `{required_feature}`"
            );
        }
    }

    #[test]
    fn malformed_fixture_is_rejected_when_slide_xml_is_parsed() {
        let manifest = load_manifest();
        let malformed = manifest
            .entries
            .iter()
            .find(|entry| entry.invariants.iter().any(|item| item == "malformed"))
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
        assert!(
            malformed
                .expected_warnings
                .iter()
                .any(|warning| warning == error.code().as_str()),
            "malformed fixture expected one of {:?}, got {}",
            malformed.expected_warnings,
            error.code().as_str()
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
            if entry.invariants.iter().any(|item| item == "malformed") {
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
