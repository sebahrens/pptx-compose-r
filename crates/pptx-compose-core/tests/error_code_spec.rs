use std::{collections::BTreeMap, fs, path::Path};

use pptx_compose_core::error::ErrorCode;

#[test]
fn spec_044_lists_every_stable_error_code_once() {
    let spec_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../specs/044-results-validation-errors.md");
    let spec = fs::read_to_string(&spec_path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", spec_path.display()));
    let spec_codes = minimum_stable_error_codes(&spec);
    let rust_codes = ErrorCode::ALL
        .into_iter()
        .map(|code| code.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        spec_codes, rust_codes,
        "specs/044 Minimum stable error codes must match ErrorCode::ALL"
    );

    let mut counts = BTreeMap::new();
    for code in spec_codes {
        *counts.entry(code).or_insert(0usize) += 1;
    }
    for code in rust_codes {
        assert_eq!(
            counts.get(code),
            Some(&1),
            "{code} must appear exactly once in specs/044"
        );
    }
}

fn minimum_stable_error_codes(spec: &str) -> Vec<&str> {
    let mut codes = Vec::new();
    let mut in_list = false;

    for line in spec.lines() {
        if line == "Minimum stable error codes:" {
            in_list = true;
            continue;
        }

        if !in_list {
            continue;
        }

        let Some(rest) = line.strip_prefix("- `") else {
            if !codes.is_empty() && line.is_empty() {
                break;
            }
            continue;
        };
        let Some((code, _)) = rest.split_once('`') else {
            continue;
        };
        codes.push(code);
    }

    assert!(
        !codes.is_empty(),
        "specs/044 Minimum stable error codes list must be present"
    );
    codes
}
