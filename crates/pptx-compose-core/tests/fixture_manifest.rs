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
