use pptx_compose_core::{
    error::Result,
    validation::{ValidationMode, validate_package},
    zip::{
        reader::from_bytes,
        writer::{WriteEntry, write_vec},
    },
};

#[path = "support/fixtures.rs"]
mod fixtures;

fn assert_no_edit_roundtrip(fixture: &fixtures::FixtureEntry) -> Result<()> {
    let input = fixtures::read_fixture_bytes(fixture);
    let original_entries = from_bytes(&input)?;
    let original_package = fixtures::package_from_entries(&original_entries)?;

    let original_validation = validate_package(&original_package, ValidationMode::NoEdit);
    fixtures::assert_valid_with_expected_warnings(fixture, &original_validation);

    let write_entries: Vec<_> = original_entries.iter().map(WriteEntry::Clean).collect();
    let output = write_vec(&input, &write_entries)?;
    let written_entries = from_bytes(&output)?;
    let written_package = fixtures::package_from_entries(&written_entries)?;

    let written_validation = validate_package(&written_package, ValidationMode::NoEdit);
    fixtures::assert_valid_with_expected_warnings(fixture, &written_validation);
    fixtures::assert_equal_part_sets(&original_entries, &written_entries, &fixture.path);
    fixtures::assert_byte_identical_parts(&original_entries, &written_entries, &fixture.path);

    Ok(())
}

mod roundtrip {
    use super::*;

    #[test]
    fn no_edit_preserve_mode_keeps_clean_parts_byte_identical() -> Result<()> {
        let manifest = fixtures::load_manifest();
        let roundtrip_fixtures = manifest.roundtrip_entries();

        assert!(
            !roundtrip_fixtures.is_empty(),
            "fixture manifest must include at least one roundtrip fixture"
        );
        assert!(
            roundtrip_fixtures
                .iter()
                .any(|entry| entry.has_feature("mc-alternate-content")
                    && entry.has_feature("unknown-part")),
            "roundtrip fixtures must cover mc:AlternateContent and unknown parts"
        );
        assert!(
            roundtrip_fixtures
                .iter()
                .any(|entry| entry.has_feature("real-world")),
            "roundtrip fixtures must cover real-world packages"
        );

        for fixture in roundtrip_fixtures {
            assert_no_edit_roundtrip(fixture)?;
        }

        Ok(())
    }
}
