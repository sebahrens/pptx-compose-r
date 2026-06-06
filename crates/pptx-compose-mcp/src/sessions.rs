use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use pptx_compose::{
    ApplyPatchOptions, PresentationDocument, WriteOptions,
    core::{
        error::{Error, ErrorCode, ErrorLocation, Result},
        opc::part_name::PartName,
        provenance::revision,
        zip::reader::from_bytes,
    },
    edit::{
        media_inputs::{MediaBinding, MediaInputs, MediaLimits, MediaSource},
        patch::Patch,
    },
    json::schemas::PatchReport,
};
use schemars::JsonSchema;
use serde::Serialize;
use sha2::{Digest, Sha256};

const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(30 * 60);
const DEFAULT_MAX_SESSIONS: usize = 32;
const DEFAULT_MAX_SESSION_MEM_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct SessionStore {
    inner: Arc<Mutex<HashMap<String, Session>>>,
    ttl: Duration,
    max_sessions: usize,
    max_session_mem_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct Session {
    pub session_id: String,
    pub document_id: String,
    pub revision: u64,
    pub package: PresentationDocument,
    pub media: HashMap<String, MediaHandle>,
    pub changed_parts: Vec<String>,
    pub expires_at: SystemTime,
    pub mem_bytes: u64,
    apply_lock: Arc<Mutex<()>>,
    next_media_index: u64,
    staged_media: HashMap<String, StagedMedia>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaHandle {
    pub media_ref: String,
    pub content_type: String,
    pub sha256: String,
    pub byte_length: u64,
    pub dimensions_px: Option<ImageDimensions>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImageDimensions {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StagedMedia {
    content_type: String,
    bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenSession {
    pub session_id: String,
    pub document_id: String,
    pub revision: u64,
    pub slide_count: u32,
    pub expires_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionConfig {
    pub ttl: Duration,
    pub max_sessions: usize,
    pub max_session_mem_bytes: u64,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::with_config(SessionConfig::default())
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            ttl: DEFAULT_SESSION_TTL,
            max_sessions: DEFAULT_MAX_SESSIONS,
            max_session_mem_bytes: DEFAULT_MAX_SESSION_MEM_BYTES,
        }
    }
}

impl SessionStore {
    #[must_use]
    pub fn with_config(config: SessionConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            ttl: config.ttl,
            max_sessions: config.max_sessions,
            max_session_mem_bytes: config.max_session_mem_bytes,
        }
    }

    pub fn open_path(&self, path: impl AsRef<Path>) -> Result<OpenSession> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|source| {
            Error::with_source(
                ErrorCode::InvalidInput,
                format!("Could not read PPTX input {}.", path.display()),
                source,
            )
        })?;
        let package = PresentationDocument::from_bytes(bytes.clone())?;
        self.open_package(package, &bytes)
    }

    pub fn open_package(&self, package: PresentationDocument, bytes: &[u8]) -> Result<OpenSession> {
        let mem_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if mem_bytes > self.max_session_mem_bytes {
            return Err(Error::resource_limit_exceeded(format!(
                "Opened PPTX is {mem_bytes} bytes, exceeding max_session_mem_bytes {}.",
                self.max_session_mem_bytes
            )));
        }

        let now = SystemTime::now();
        let expires_at = now.checked_add(self.ttl).ok_or_else(|| {
            Error::new(
                ErrorCode::InternalError,
                "Session TTL overflowed system time.",
            )
        })?;
        let metadata = package_metadata(&package, bytes)?;
        let session_id = unique_prefixed_id("sess");
        let revision = revision::on_open().value();
        let session = Session {
            session_id: session_id.clone(),
            document_id: metadata.document_id.clone(),
            revision,
            package,
            media: HashMap::new(),
            changed_parts: Vec::new(),
            expires_at,
            mem_bytes,
            apply_lock: Arc::new(Mutex::new(())),
            next_media_index: 1,
            staged_media: HashMap::new(),
        };

        let mut sessions = self.lock_sessions()?;
        remove_expired(&mut sessions, now);
        if sessions.len() >= self.max_sessions {
            return Err(Error::resource_limit_exceeded(format!(
                "Session limit {} has been reached.",
                self.max_sessions
            )));
        }
        sessions.insert(session_id.clone(), session);

        Ok(OpenSession {
            session_id,
            document_id: metadata.document_id,
            revision,
            slide_count: metadata.slide_count,
            expires_at: system_time_json(expires_at),
        })
    }

    pub fn close(&self, session_id: &str) -> Result<bool> {
        let mut sessions = self.lock_sessions()?;
        remove_expired(&mut sessions, SystemTime::now());
        Ok(sessions.remove(session_id).is_some())
    }

    pub fn get(&self, session_id: &str) -> Result<Session> {
        let mut sessions = self.lock_sessions()?;
        remove_expired(&mut sessions, SystemTime::now());
        sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| missing_session(session_id))
    }

    pub fn check_revision(&self, session_id: &str, expected_revision: u64) -> Result<u64> {
        let mut sessions = self.lock_sessions()?;
        remove_expired(&mut sessions, SystemTime::now());
        let session = sessions
            .get(session_id)
            .ok_or_else(|| missing_session(session_id))?;
        if session.revision != expected_revision {
            return Err(stale_revision_error(
                session_id,
                session.revision,
                expected_revision,
            ));
        }
        Ok(session.revision)
    }

    pub fn check_patch_envelope(&self, session_id: &str, patch: &Patch) -> Result<u64> {
        let mut sessions = self.lock_sessions()?;
        remove_expired(&mut sessions, SystemTime::now());
        let session = sessions
            .get(session_id)
            .ok_or_else(|| missing_session(session_id))?;
        if session.document_id != patch.document_id {
            return Err(stale_document_error(session_id, session.revision));
        }
        if session.revision != u64::from(patch.base_revision) {
            return Err(stale_revision_error(
                session_id,
                session.revision,
                u64::from(patch.base_revision),
            ));
        }
        Ok(session.revision)
    }

    pub fn record_apply(
        &self,
        session_id: &str,
        expected_revision: u64,
        dry_run: bool,
        succeeded: bool,
    ) -> Result<u64> {
        let session = self.get(session_id)?;
        let _apply_guard = session.apply_lock.lock().map_err(|_| {
            Error::new(
                ErrorCode::InternalError,
                "Session apply lock was poisoned by a previous failure.",
            )
        })?;

        let mut sessions = self.lock_sessions()?;
        remove_expired(&mut sessions, SystemTime::now());
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| missing_session(session_id))?;
        if session.revision != expected_revision {
            return Err(stale_revision_error(
                session_id,
                session.revision,
                expected_revision,
            ));
        }
        if succeeded && !dry_run {
            session.revision = session.revision.saturating_add(1);
        }
        Ok(session.revision)
    }

    pub fn import_media_path(
        &self,
        session_id: &str,
        media_path: impl AsRef<Path>,
        content_type: &str,
    ) -> Result<MediaHandle> {
        let media_path = media_path.as_ref();
        let bytes = fs::read(media_path).map_err(|source| {
            Error::with_source(
                ErrorCode::InvalidInput,
                format!("Could not read media input {}.", media_path.display()),
                source,
            )
        })?;
        self.import_media_bytes(session_id, bytes, content_type)
    }

    pub fn import_media_bytes(
        &self,
        session_id: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<MediaHandle> {
        let byte_length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let mut sessions = self.lock_sessions()?;
        remove_expired(&mut sessions, SystemTime::now());
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| missing_session(session_id))?;
        let next_mem = session.mem_bytes.checked_add(byte_length).ok_or_else(|| {
            Error::resource_limit_exceeded("Session memory accounting overflowed.")
        })?;
        if next_mem > self.max_session_mem_bytes {
            return Err(Error::resource_limit_exceeded(format!(
                "Session {session_id} would use {next_mem} bytes, exceeding max_session_mem_bytes {}.",
                self.max_session_mem_bytes
            )));
        }

        let media_ref = format!("media_{}", session.next_media_index);
        let inputs = MediaInputs::with_limits(
            HashMap::from([(
                media_ref.clone(),
                MediaBinding {
                    content_type: content_type.to_owned(),
                    declared_sha256: None,
                    declared_byte_length: Some(byte_length),
                    source: MediaSource::Bytes(bytes.clone()),
                },
            )]),
            MediaLimits {
                max_media_bytes: self.max_session_mem_bytes,
            },
        );
        let resolved = inputs.resolve(&media_ref)?;
        let handle = MediaHandle {
            media_ref: media_ref.clone(),
            content_type: resolved.content_type,
            sha256: resolved.sha256,
            byte_length,
            dimensions_px: image_dimensions(content_type, &bytes),
        };

        session.mem_bytes = next_mem;
        session.next_media_index = session.next_media_index.saturating_add(1);
        session.staged_media.insert(
            media_ref.clone(),
            StagedMedia {
                content_type: content_type.to_owned(),
                bytes,
            },
        );
        session.media.insert(media_ref, handle.clone());
        Ok(handle)
    }

    pub fn validate_patch(&self, session_id: &str, patch: Patch) -> Result<PatchReport> {
        self.check_patch_envelope(session_id, &patch)?;
        let session = self.get(session_id)?;
        session.package.clone().apply_patch_with_options(
            patch,
            media_inputs(&session),
            ApplyPatchOptions {
                dry_run: true,
                validate: true,
            },
        )
    }

    pub fn apply_patch(
        &self,
        session_id: &str,
        patch: Patch,
        dry_run: bool,
    ) -> Result<ApplyResult> {
        let apply_lock = self.get(session_id)?.apply_lock.clone();
        let _apply_guard = apply_lock.lock().map_err(|_| {
            Error::new(
                ErrorCode::InternalError,
                "Session apply lock was poisoned by a previous failure.",
            )
        })?;

        let expected_revision = self.check_patch_envelope(session_id, &patch)?;
        let (mut package, media) = {
            let mut sessions = self.lock_sessions()?;
            remove_expired(&mut sessions, SystemTime::now());
            let session = sessions
                .get(session_id)
                .ok_or_else(|| missing_session(session_id))?;
            if session.revision != expected_revision {
                return Err(stale_revision_error(
                    session_id,
                    session.revision,
                    expected_revision,
                ));
            }
            (session.package.clone(), media_inputs(session))
        };

        let report = package.apply_patch_with_options(
            patch,
            media,
            ApplyPatchOptions {
                dry_run,
                validate: true,
            },
        )?;

        let mut sessions = self.lock_sessions()?;
        remove_expired(&mut sessions, SystemTime::now());
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| missing_session(session_id))?;
        if session.revision != expected_revision {
            return Err(stale_revision_error(
                session_id,
                session.revision,
                expected_revision,
            ));
        }
        if !dry_run {
            session.package = package;
            session.revision = u64::from(report.new_revision);
            session
                .changed_parts
                .extend(report.changed_parts.iter().cloned());
            session.changed_parts.sort();
            session.changed_parts.dedup();
        }

        Ok(ApplyResult {
            revision: session.revision,
            report,
        })
    }

    pub fn changed_parts(&self, session_id: &str) -> Result<Vec<String>> {
        let session = self.get(session_id)?;
        Ok(session.changed_parts)
    }

    pub fn export_bytes(
        &self,
        session_id: &str,
        expected_revision: Option<u64>,
    ) -> Result<Vec<u8>> {
        let session = self.get(session_id)?;
        check_optional_revision(session_id, session.revision, expected_revision)?;
        session.package.write_vec()
    }

    pub fn export_path(
        &self,
        session_id: &str,
        expected_revision: Option<u64>,
        path: impl AsRef<Path>,
        overwrite: bool,
    ) -> Result<u64> {
        let session = self.get(session_id)?;
        check_optional_revision(session_id, session.revision, expected_revision)?;
        session.package.write_path_with_options(
            path,
            WriteOptions {
                overwrite,
                ..WriteOptions::default()
            },
        )?;
        u64::try_from(session.package.write_vec()?.len()).map_err(|source| {
            Error::with_source(
                ErrorCode::InternalError,
                "Exported PPTX length exceeds reportable range.",
                source,
            )
        })
    }

    fn lock_sessions(&self) -> Result<MutexGuard<'_, HashMap<String, Session>>> {
        self.inner.lock().map_err(|_| {
            Error::new(
                ErrorCode::InternalError,
                "Session store lock was poisoned by a previous failure.",
            )
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ApplyResult {
    pub revision: u64,
    pub report: PatchReport,
}

fn media_inputs(session: &Session) -> MediaInputs {
    MediaInputs::with_limits(
        session
            .staged_media
            .iter()
            .map(|(media_ref, media)| {
                (
                    media_ref.clone(),
                    MediaBinding {
                        content_type: media.content_type.clone(),
                        declared_sha256: None,
                        declared_byte_length: u64::try_from(media.bytes.len()).ok(),
                        source: MediaSource::Bytes(media.bytes.clone()),
                    },
                )
            })
            .collect(),
        MediaLimits {
            max_media_bytes: session.mem_bytes,
        },
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PackageMetadata {
    document_id: String,
    slide_count: u32,
}

fn package_metadata(package: &PresentationDocument, bytes: &[u8]) -> Result<PackageMetadata> {
    let entries = from_bytes(bytes)?;
    let mut content_types = None;
    let mut slide_count = 0_u32;
    for entry in entries {
        if entry.meta.is_dir {
            continue;
        }
        let part_name = PartName::from_zip_entry(entry.meta.original_name.as_str())?;
        if part_name.as_str() == "/[Content_Types].xml" {
            content_types = Some(entry.bytes.clone());
        }
        if is_slide_part(part_name.as_str()) {
            slide_count = slide_count.saturating_add(1);
        }
    }
    let _content_types = content_types
        .ok_or_else(|| Error::unsupported_package("Package is missing [Content_Types].xml."))?;
    let validation = package.validate()?;

    Ok(PackageMetadata {
        document_id: validation.document_id,
        slide_count,
    })
}

fn is_slide_part(part_name: &str) -> bool {
    let Some(file_name) = part_name.strip_prefix("/ppt/slides/slide") else {
        return false;
    };
    let Some(index) = file_name.strip_suffix(".xml") else {
        return false;
    };
    !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
}

fn remove_expired(sessions: &mut HashMap<String, Session>, now: SystemTime) {
    sessions.retain(|_, session| session.expires_at > now);
}

fn missing_session(session_id: &str) -> Error {
    Error::new(
        ErrorCode::InvalidInput,
        format!("Session {session_id} does not exist or has expired."),
    )
}

fn stale_revision_error(session_id: &str, current_revision: u64, expected_revision: u64) -> Error {
    Error::stale_revision(format!(
        "Session {session_id} is at revision {current_revision}, not expected revision {expected_revision}."
    ))
    .with_location(ErrorLocation {
        current_revision: Some(current_revision),
        ..ErrorLocation::default()
    })
}

fn stale_document_error(session_id: &str, current_revision: u64) -> Error {
    Error::stale_revision(format!(
        "Patch document_id does not match session {session_id}."
    ))
    .with_location(ErrorLocation {
        current_revision: Some(current_revision),
        ..ErrorLocation::default()
    })
}

fn check_optional_revision(
    session_id: &str,
    current_revision: u64,
    expected_revision: Option<u64>,
) -> Result<()> {
    if let Some(expected_revision) = expected_revision
        && current_revision != expected_revision
    {
        return Err(stale_revision_error(
            session_id,
            current_revision,
            expected_revision,
        ));
    }
    Ok(())
}

fn unique_prefixed_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{prefix}_{nanos:x}_{counter:x}")
}

fn system_time_json(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    unix_seconds_rfc3339(seconds)
}

fn unix_seconds_rfc3339(seconds: u64) -> String {
    let days = seconds / 86_400;
    let second_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_unix_days(days);
    let hour = second_of_day / 3_600;
    let minute = (second_of_day % 3_600) / 60;
    let second = second_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_unix_days(days: u64) -> (i64, u64, u64) {
    let z = i128::from(days) + 719_468;
    let era = z / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (
        i64::try_from(year).unwrap_or(i64::MAX),
        u64::try_from(month).unwrap_or(1),
        u64::try_from(day).unwrap_or(1),
    )
}

fn image_dimensions(content_type: &str, bytes: &[u8]) -> Option<ImageDimensions> {
    match content_type {
        "image/png" => png_dimensions(bytes),
        "image/jpeg" => jpeg_dimensions(bytes),
        "image/gif" => gif_dimensions(bytes),
        _ => None,
    }
}

fn png_dimensions(bytes: &[u8]) -> Option<ImageDimensions> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    Some(ImageDimensions {
        width: u32::from_be_bytes(bytes[16..20].try_into().ok()?),
        height: u32::from_be_bytes(bytes[20..24].try_into().ok()?),
    })
}

fn gif_dimensions(bytes: &[u8]) -> Option<ImageDimensions> {
    if bytes.len() < 10 || !matches!(&bytes[..6], b"GIF87a" | b"GIF89a") {
        return None;
    }
    Some(ImageDimensions {
        width: u16::from_le_bytes(bytes[6..8].try_into().ok()?).into(),
        height: u16::from_le_bytes(bytes[8..10].try_into().ok()?).into(),
    })
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<ImageDimensions> {
    if bytes.len() < 4 || bytes[0] != 0xff || bytes[1] != 0xd8 {
        return None;
    }
    let mut offset = 2;
    while offset + 9 < bytes.len() {
        if bytes[offset] != 0xff {
            return None;
        }
        let marker = bytes[offset + 1];
        offset += 2;
        while marker == 0xff && offset < bytes.len() {
            offset += 1;
        }
        if matches!(marker, 0xd8 | 0xd9) {
            continue;
        }
        if offset + 2 > bytes.len() {
            return None;
        }
        let segment_len = usize::from(u16::from_be_bytes(
            bytes[offset..offset + 2].try_into().ok()?,
        ));
        if segment_len < 2 || offset + segment_len > bytes.len() {
            return None;
        }
        if matches!(
            marker,
            0xc0 | 0xc1
                | 0xc2
                | 0xc3
                | 0xc5
                | 0xc6
                | 0xc7
                | 0xc9
                | 0xca
                | 0xcb
                | 0xcd
                | 0xce
                | 0xcf
        ) {
            if segment_len < 7 {
                return None;
            }
            return Some(ImageDimensions {
                height: u16::from_be_bytes(bytes[offset + 3..offset + 5].try_into().ok()?).into(),
                width: u16::from_be_bytes(bytes[offset + 5..offset + 7].try_into().ok()?).into(),
            });
        }
        offset += segment_len;
    }
    None
}

#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity("sha256:".len() + 64);
    output.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String succeeds");
    }
    output
}

#[cfg(test)]
#[test]
fn system_time_json_formats_expires_at_as_rfc3339_utc() {
    let formatted = system_time_json(UNIX_EPOCH + Duration::from_secs(1_749_081_600));

    assert_eq!(formatted, "2025-06-05T00:00:00Z");
    assert!(is_rfc3339_utc_seconds(&formatted));
}

#[cfg(test)]
#[test]
fn revision_increments_on_apply() {
    let store = SessionStore::default();
    let opened = store
        .open_package(test_empty_deck(), &test_minimal_pptx_bytes())
        .expect("session opens");

    assert_eq!(opened.revision, 1);
    assert_eq!(
        store
            .record_apply(&opened.session_id, opened.revision, false, true)
            .expect("successful mutation increments"),
        2
    );
    assert_eq!(
        store
            .record_apply(&opened.session_id, 2, true, true)
            .expect("dry-run keeps revision"),
        2
    );
}

#[cfg(test)]
#[test]
fn concurrent_same_revision_applies_serialize_and_reject_stale_patch() {
    let store = SessionStore::default();
    let fixture = test_text_pptx_bytes();
    let opened = store
        .open_package(
            PresentationDocument::from_bytes(fixture.clone()).expect("fixture pptx opens"),
            &fixture,
        )
        .expect("session opens");
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let first = replace_text_patch(&opened.document_id, opened.revision, "Concurrent title A");
    let second = replace_text_patch(&opened.document_id, opened.revision, "Concurrent title B");

    let first_thread = spawn_concurrent_apply(
        store.clone(),
        opened.session_id.clone(),
        first,
        barrier.clone(),
    );
    let second_thread = spawn_concurrent_apply(
        store.clone(),
        opened.session_id.clone(),
        second,
        barrier.clone(),
    );

    barrier.wait();
    let outcomes = [
        first_thread.join().expect("first apply thread completes"),
        second_thread.join().expect("second apply thread completes"),
    ];
    let applied = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, ApplyOutcome::Applied(2)))
        .count();
    let stale = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, ApplyOutcome::Failed(ErrorCode::StalePatch)))
        .count();

    assert_eq!(applied, 1, "exactly one apply mutates and increments");
    assert_eq!(stale, 1, "the same-revision loser is stale");
    assert_eq!(
        store
            .get(&opened.session_id)
            .expect("session remains open")
            .revision,
        2
    );
}

#[cfg(test)]
#[test]
fn session_ttl_eviction() {
    let store = SessionStore::with_config(SessionConfig {
        ttl: Duration::from_millis(1),
        max_sessions: 8,
        max_session_mem_bytes: DEFAULT_MAX_SESSION_MEM_BYTES,
    });
    let opened = store
        .open_package(test_empty_deck(), &test_minimal_pptx_bytes())
        .expect("session opens");

    std::thread::sleep(Duration::from_millis(5));
    let error = store
        .get(&opened.session_id)
        .expect_err("expired session rejected");
    assert_eq!(error.code(), ErrorCode::InvalidInput);

    let reopened = store
        .open_package(test_empty_deck(), &test_minimal_pptx_bytes())
        .expect("expired session was cleaned up");
    assert_ne!(opened.session_id, reopened.session_id);
}

#[cfg(test)]
fn is_rfc3339_utc_seconds(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'Z'
        && bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19) || byte.is_ascii_digit()
        })
}

#[cfg(test)]
fn test_empty_deck() -> PresentationDocument {
    PresentationDocument::from_bytes(test_minimal_pptx_bytes()).expect("fixture pptx opens")
}

#[cfg(test)]
fn test_minimal_pptx_bytes() -> Vec<u8> {
    use std::io::{Cursor, Write};

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("[Content_Types].xml", options)
            .expect("content types starts");
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#,
        )
        .expect("content types writes");
        zip.finish().expect("zip finishes");
    }
    cursor.into_inner()
}

#[cfg(test)]
#[derive(Debug, Eq, PartialEq)]
enum ApplyOutcome {
    Applied(u64),
    Failed(ErrorCode),
}

#[cfg(test)]
fn spawn_concurrent_apply(
    store: SessionStore,
    session_id: String,
    patch: Patch,
    barrier: Arc<std::sync::Barrier>,
) -> std::thread::JoinHandle<ApplyOutcome> {
    std::thread::spawn(move || {
        barrier.wait();
        match store.apply_patch(&session_id, patch, false) {
            Ok(result) => ApplyOutcome::Applied(result.revision),
            Err(error) => ApplyOutcome::Failed(error.code()),
        }
    })
}

#[cfg(test)]
fn replace_text_patch(document_id: &str, base_revision: u64, text: &str) -> Patch {
    let value = serde_json::json!({
        "schema": pptx_compose::edit::patch::PATCH_SCHEMA,
        "version": pptx_compose::edit::patch::PATCH_VERSION,
        "document_id": document_id,
        "base_revision": u32::try_from(base_revision).expect("test fixture revision fits u32"),
        "client_request_id": "test-request",
        "operations": [{
            "operation_id": "replace-title",
            "op": "replace_text",
            "element_id": "slide-1:shape-3",
            "text": text
        }]
    });
    serde_json::from_value(value).expect("test patch deserializes")
}

#[cfg(test)]
fn test_text_pptx_bytes() -> Vec<u8> {
    use std::io::{Cursor, Write};
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    let content_types = test_text_content_types();
    let root_rels = test_text_root_rels();
    let presentation = test_text_presentation();
    let presentation_rels = test_text_presentation_rels();
    let slide = test_text_slide();
    let entries = [
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", root_rels.as_bytes()),
        ("ppt/presentation.xml", presentation.as_bytes()),
        (
            "ppt/_rels/presentation.xml.rels",
            presentation_rels.as_bytes(),
        ),
        ("ppt/slides/slide1.xml", slide.as_bytes()),
    ];
    let mut bytes = Vec::new();
    {
        let mut writer = ZipWriter::new(Cursor::new(&mut bytes));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, data) in entries {
            writer.start_file(name, options).expect("start ZIP entry");
            writer.write_all(data).expect("write ZIP entry");
        }
        writer.finish().expect("finish ZIP");
    }
    bytes
}

#[cfg(test)]
fn test_text_content_types() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
</Types>"#
        .to_owned()
}

#[cfg(test)]
fn test_text_root_rels() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>"#
        .to_owned()
}

#[cfg(test)]
fn test_text_presentation() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
</p:presentation>"#
        .to_owned()
}

#[cfg(test)]
fn test_text_presentation_rels() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
</Relationships>"#
        .to_owned()
}

#[cfg(test)]
fn test_text_slide() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="Title"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="914400" y="457200"/><a:ext cx="3657600" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
        <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Original title</a:t></a:r></a:p></p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
</p:sld>"#
        .to_owned()
}
