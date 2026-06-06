use std::{fs, io::Cursor, path::Path};

use pptx_compose::{
    AgentViewOptions, ApplyPatchOptions, MediaInputs, OpenOptions, Patch, PresentationDocument,
    WriteMode, WriteOptions,
};

#[test]
fn exposes_required_070_api_and_defaults() {
    let defaults = WriteOptions::default();
    assert_eq!(
        defaults,
        WriteOptions {
            mode: WriteMode::Preserve,
            overwrite: false,
            validate: true,
            atomic: true,
        }
    );

    let bytes = include_bytes!("../../../fixtures/minimal.pptx");
    let mut from_bytes = PresentationDocument::from_bytes(bytes).expect("fixture opens");
    let _from_reader = PresentationDocument::open_reader(Cursor::new(bytes)).expect("reader opens");

    let root = unique_dir();
    let input = root.join("input.pptx");
    let output = root.join("output.pptx");
    fs::write(&input, bytes).expect("fixture writes");
    let from_path = PresentationDocument::open_path(&input).expect("path opens");
    let _from_path_with_options =
        PresentationDocument::open_path_with_options(&input, OpenOptions::default())
            .expect("path opens with options");

    let _agent_json = from_path.to_agent_json().expect("agent view builds");
    let _agent_json_with_options = from_path
        .to_agent_json_with_options(AgentViewOptions::summary())
        .expect("agent view builds with options");
    let _legacy_json = from_path.to_legacy_json().expect("legacy JSON builds");
    let _validation = from_path.validate().expect("validation report builds");
    let _bytes = from_path.write_vec().expect("default write vec succeeds");
    let _bytes_with_options = from_path
        .write_vec_with_options(WriteOptions::default())
        .expect("write vec with options succeeds");
    from_path
        .write_path_with_options(
            &output,
            WriteOptions {
                overwrite: false,
                ..WriteOptions::default()
            },
        )
        .expect("write path with options succeeds");

    let report = from_bytes
        .apply_patch(noop_patch(bytes), MediaInputs::default())
        .expect("apply_patch returns a report");
    assert_eq!(
        report.status,
        pptx_compose::json::schemas::PatchStatus::Applied
    );
    let report = from_bytes
        .apply_patch_with_options(
            noop_patch(bytes),
            MediaInputs::default(),
            ApplyPatchOptions {
                dry_run: true,
                validate: true,
            },
        )
        .expect("apply_patch_with_options returns a report");
    assert_eq!(
        report.status,
        pptx_compose::json::schemas::PatchStatus::DryRunSuccess
    );

    let output_default = root.join("default-output.pptx");
    from_bytes
        .write_path(&output_default)
        .expect("default write path succeeds");

    fs::remove_dir_all(root).expect("test dir removes");
}

fn noop_patch(bytes: &[u8]) -> Patch {
    serde_json::from_value(serde_json::json!({
        "schema": "pptx-compose.patch.v1",
        "version": 1,
        "document_id": document_id(bytes),
        "base_revision": 1,
        "client_request_id": "facade-api-noop",
        "operations": []
    }))
    .expect("noop patch parses")
}

fn document_id(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(bytes);
    let mut output = String::from("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String succeeds");
    }
    output
}

fn unique_dir() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    let base_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    let temp_dir = std::env::temp_dir();

    for _ in 0..100 {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = temp_dir.join(format!(
            "pptx-compose-facade-api-{}-{base_nanos}-{id}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return path,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("test dir creates: {error}"),
        }
    }

    panic!("could not create a unique facade test directory")
}

#[allow(dead_code)]
fn _assert_no_internal_parser_types_in_primary_api(_path: &Path) {}
