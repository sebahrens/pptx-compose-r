use std::{
    collections::BTreeMap,
    io::{Cursor, Read, Seek, Write},
};

use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::{
    error::{Error, ErrorCode, Result},
    opc::{package::Package, part_name::PartName},
    validation::{ValidationMode, validate_package},
    zip::{ZipEntryMetadata, reader::RawEntry},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriteOptions {
    pub mode: WriteMode,
    pub validate_on_write: bool,
    pub compression_method: zip::CompressionMethod,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            mode: WriteMode::Preserve,
            validate_on_write: true,
            compression_method: zip::CompressionMethod::Deflated,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WriteMode {
    #[default]
    Preserve,
    Deterministic,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteEntry<'a> {
    Clean(&'a RawEntry),
    Dirty(DirtyEntry<'a>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirtyEntry<'a> {
    pub name: &'a str,
    pub bytes: &'a [u8],
    pub meta: &'a ZipEntryMetadata,
}

pub fn write_package_vec(package: &Package, options: &WriteOptions) -> Result<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    write_package_preserve(package, &mut output, options)?;
    Ok(output.into_inner())
}

pub fn write_package_preserve<W>(package: &Package, output: W, options: &WriteOptions) -> Result<W>
where
    W: Write + Seek,
{
    if options.validate_on_write {
        validate_write(package)?;
    }

    let dirty_metas = dirty_metadata_for_package(package);
    let entries = write_entries_for_package(package, &dirty_metas);
    let mut writer = PackageZipWriter::new(output, options);
    for entry in entries {
        let WriteEntry::Dirty(entry) = entry else {
            continue;
        };
        writer.write_dirty(entry.name, entry.bytes, entry.meta)?;
    }
    writer.finish()
}

pub fn write_vec(source_package: &[u8], entries: &[WriteEntry<'_>]) -> Result<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    write_writer(source_package, entries, &mut output)?;
    Ok(output.into_inner())
}

pub fn write_writer<W>(source_package: &[u8], entries: &[WriteEntry<'_>], output: W) -> Result<W>
where
    W: Write + Seek,
{
    write_writer_with_options(source_package, entries, output, &WriteOptions::default())
}

pub fn write_writer_with_options<W>(
    source_package: &[u8],
    entries: &[WriteEntry<'_>],
    output: W,
    options: &WriteOptions,
) -> Result<W>
where
    W: Write + Seek,
{
    let mut source = ZipArchive::new(Cursor::new(source_package))
        .map_err(|source| Error::parse_error("Could not open source ZIP package.", source))?;
    let mut writer = PackageZipWriter::new(output, options);

    let ordered_entries = ordered_entries(entries, options.mode);
    for entry in ordered_entries {
        match entry {
            WriteEntry::Clean(entry) => writer.write_clean(&mut source, entry)?,
            WriteEntry::Dirty(entry) => writer.write_dirty(entry.name, entry.bytes, entry.meta)?,
        }
    }

    writer.finish()
}

pub struct PackageZipWriter<'a, W>
where
    W: Write + Seek,
{
    writer: ZipWriter<W>,
    options: &'a WriteOptions,
}

impl<'a, W> PackageZipWriter<'a, W>
where
    W: Write + Seek,
{
    pub fn new(output: W, options: &'a WriteOptions) -> Self {
        Self {
            writer: ZipWriter::new(output),
            options,
        }
    }

    pub fn write_clean<R>(&mut self, source: &mut ZipArchive<R>, entry: &RawEntry) -> Result<()>
    where
        R: Read + Seek,
    {
        let source_file = source.by_index(entry.meta.entry_index).map_err(|source| {
            Error::parse_error("Could not find clean ZIP entry in source package.", source)
        })?;
        if source_file.name() != entry.meta.original_name {
            return Err(Error::new(
                ErrorCode::WriteFailed,
                format!(
                    "Source ZIP entry {} no longer matches expected entry {}.",
                    source_file.name(),
                    entry.meta.original_name
                ),
            ));
        }

        if self.options.mode == WriteMode::Deterministic {
            self.writer
                .raw_copy_file_touch(
                    source_file,
                    deterministic_timestamp(),
                    entry.meta.external_attrs,
                )
                .map_err(|source| {
                    Error::with_source(
                        ErrorCode::WriteFailed,
                        "Could not raw-copy clean ZIP entry with deterministic metadata.",
                        source,
                    )
                })
        } else {
            self.writer.raw_copy_file(source_file).map_err(|source| {
                Error::with_source(
                    ErrorCode::WriteFailed,
                    "Could not raw-copy clean ZIP entry.",
                    source,
                )
            })
        }
    }

    pub fn write_dirty(&mut self, name: &str, bytes: &[u8], meta: &ZipEntryMetadata) -> Result<()> {
        let mut file_options = SimpleFileOptions::default()
            .compression_method(self.compression_method(meta))
            .large_file(bytes.len() > u32::MAX as usize);
        if let Some(last_modified) = self.last_modified_time(meta) {
            file_options = file_options.last_modified_time(last_modified);
        }
        if let Some(external_attrs) = meta.external_attrs {
            file_options = file_options.unix_permissions(external_attrs);
        }

        if meta.is_dir {
            self.writer
                .add_directory(name.trim_end_matches('/'), file_options)
                .map_err(|source| {
                    Error::with_source(
                        ErrorCode::WriteFailed,
                        "Could not write dirty ZIP directory.",
                        source,
                    )
                })?;
        } else {
            self.writer
                .start_file(name, file_options)
                .map_err(|source| {
                    Error::with_source(
                        ErrorCode::WriteFailed,
                        "Could not start dirty ZIP entry.",
                        source,
                    )
                })?;
            self.writer.write_all(bytes).map_err(|source| {
                Error::with_source(
                    ErrorCode::WriteFailed,
                    "Could not write dirty ZIP entry bytes.",
                    source,
                )
            })?;
        }

        Ok(())
    }

    fn compression_method(&self, meta: &ZipEntryMetadata) -> zip::CompressionMethod {
        match self.options.mode {
            WriteMode::Preserve => meta.compression_method,
            WriteMode::Deterministic => {
                if meta.is_dir {
                    zip::CompressionMethod::Stored
                } else {
                    self.options.compression_method
                }
            }
        }
    }

    fn last_modified_time(&self, meta: &ZipEntryMetadata) -> Option<zip::DateTime> {
        match self.options.mode {
            WriteMode::Preserve => meta.last_modified.filter(zip::DateTime::is_valid),
            WriteMode::Deterministic => Some(deterministic_timestamp()),
        }
    }

    pub fn finish(self) -> Result<W> {
        self.writer.finish().map_err(|source| {
            Error::with_source(
                ErrorCode::WriteFailed,
                "Could not finish ZIP package.",
                source,
            )
        })
    }
}

fn ordered_entries<'a>(entries: &'a [WriteEntry<'a>], mode: WriteMode) -> Vec<&'a WriteEntry<'a>> {
    let mut ordered_entries: Vec<_> = entries.iter().collect();
    if mode == WriteMode::Deterministic {
        ordered_entries.sort_by(|left, right| {
            deterministic_order_key(left).cmp(&deterministic_order_key(right))
        });
    }
    ordered_entries
}

fn deterministic_order_key<'a>(entry: &'a WriteEntry<'_>) -> (u8, &'a str) {
    let name = entry.name();
    (deterministic_control_order(name), name)
}

fn deterministic_control_order(name: &str) -> u8 {
    let name = name.strip_prefix('/').unwrap_or(name);
    if name == "[Content_Types].xml" {
        0
    } else if name == "_rels/.rels" {
        1
    } else if name.ends_with(".rels") && name.contains("/_rels/") {
        2
    } else {
        3
    }
}

impl WriteEntry<'_> {
    fn name(&self) -> &str {
        match self {
            WriteEntry::Clean(entry) => entry.meta.original_name.as_str(),
            WriteEntry::Dirty(entry) => entry.name,
        }
    }

    fn meta(&self) -> &ZipEntryMetadata {
        match self {
            WriteEntry::Clean(entry) => &entry.meta,
            WriteEntry::Dirty(entry) => entry.meta,
        }
    }
}

fn validate_write(package: &Package) -> Result<()> {
    let mode = if package.dirty_parts().is_empty() {
        ValidationMode::NoEdit
    } else {
        ValidationMode::Edited
    };
    let outcome = validate_package(package, mode);
    if !outcome.findings.iter().any(|finding| finding.blocking) {
        return Ok(());
    }

    Err(Error::new(
        ErrorCode::ValidationFailed,
        format!(
            "Package validation blocked write: {} fatal findings and {} error findings.",
            outcome.summary.fatal, outcome.summary.errors
        ),
    ))
}

fn write_entries_for_package<'a>(
    package: &'a Package,
    dirty_metas: &'a BTreeMap<PartName, ZipEntryMetadata>,
) -> Vec<WriteEntry<'a>> {
    let mut entries = package
        .parts()
        .iter()
        .filter_map(|part| {
            dirty_metas.get(part.name()).map(|meta| {
                WriteEntry::Dirty(DirtyEntry {
                    name: part.original_zip_entry_name(),
                    bytes: part.bytes(),
                    meta,
                })
            })
        })
        .collect::<Vec<_>>();

    entries.sort_by(|left, right| {
        left.meta()
            .entry_index
            .cmp(&right.meta().entry_index)
            .then_with(|| left.name().cmp(right.name()))
    });
    entries
}

fn dirty_metadata_for_package(package: &Package) -> BTreeMap<PartName, ZipEntryMetadata> {
    package
        .parts()
        .iter()
        .enumerate()
        .map(|(fallback_index, part)| {
            let mut meta = part.zip_metadata().clone();
            meta.uncompressed_size = part.bytes().len() as u64;
            if meta.entry_index == usize::MAX {
                meta = dirty_zip_metadata(
                    fallback_index,
                    part.original_zip_entry_name(),
                    part.bytes(),
                );
            }
            (part.name().clone(), meta)
        })
        .collect()
}

fn dirty_zip_metadata(index: usize, name: &str, bytes: &[u8]) -> ZipEntryMetadata {
    ZipEntryMetadata {
        entry_index: index,
        original_name: name.to_owned(),
        compression_method: zip::CompressionMethod::Deflated,
        crc32: 0,
        compressed_size: 0,
        uncompressed_size: bytes.len() as u64,
        last_modified: None,
        external_attrs: None,
        is_dir: false,
    }
}

fn deterministic_timestamp() -> zip::DateTime {
    zip::DateTime::DEFAULT
}

#[cfg(test)]
#[test]
fn raw_copy_is_byte_identical() {
    use std::io::Cursor;
    use zip::ZipArchive;

    use crate::zip::reader::from_bytes;

    let package = include_bytes!("../../../../fixtures/minimal.pptx");
    let entries = from_bytes(package).expect("minimal fixture reads");
    let write_entries: Vec<_> = entries.iter().map(WriteEntry::Clean).collect();

    let written_package = write_vec(package, &write_entries).expect("clean entries write");

    let mut original_archive = ZipArchive::new(Cursor::new(package)).expect("original opens");
    let mut written_archive =
        ZipArchive::new(Cursor::new(&written_package)).expect("written opens");
    assert_eq!(written_archive.len(), original_archive.len());

    for index in 0..original_archive.len() {
        let original = original_archive
            .by_index(index)
            .expect("original entry exists");
        let written = written_archive
            .by_index(index)
            .expect("written entry exists");

        assert_eq!(written.name(), original.name());
        assert_eq!(written.crc32(), original.crc32());
        assert_eq!(written.compressed_size(), original.compressed_size());
        assert_eq!(written.size(), original.size());
        assert_eq!(
            compressed_bytes(&written_package, index),
            compressed_bytes(package, index)
        );
    }
}

#[cfg(test)]
#[test]
fn deterministic_mode_is_byte_stable_across_runs() {
    use std::io::Cursor;
    use zip::ZipArchive;

    use crate::zip::reader::from_bytes;

    let package = include_bytes!("../../../../fixtures/minimal.pptx");
    let entries = from_bytes(package).expect("minimal fixture reads");
    let write_entries: Vec<_> = entries.iter().rev().map(WriteEntry::Clean).collect();
    let options = WriteOptions {
        mode: WriteMode::Deterministic,
        ..WriteOptions::default()
    };

    let first = write_vec_with_options(package, &write_entries, &options)
        .expect("first deterministic write succeeds");
    let second = write_vec_with_options(package, &write_entries, &options)
        .expect("second deterministic write succeeds");

    assert_eq!(first, second);

    let mut original_archive = ZipArchive::new(Cursor::new(package)).expect("original opens");
    let mut written_archive = ZipArchive::new(Cursor::new(&first)).expect("written opens");
    assert_eq!(written_archive.len(), original_archive.len());

    let original_by_name = (0..original_archive.len())
        .map(|index| {
            let original = original_archive
                .by_index(index)
                .expect("original entry exists");
            (
                original.name().to_owned(),
                (
                    original.crc32(),
                    original.compressed_size(),
                    original.size(),
                    compressed_bytes(package, index).to_vec(),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let expected_names = [
        "[Content_Types].xml",
        "_rels/.rels",
        "ppt/_rels/presentation.xml.rels",
        "ppt/presentation.xml",
        "ppt/slides/slide1.xml",
    ];

    for (index, expected_name) in expected_names.iter().enumerate() {
        let written = written_archive
            .by_index(index)
            .expect("written entry exists");
        let original = original_by_name
            .get(*expected_name)
            .expect("original entry exists");

        assert_eq!(written.name(), *expected_name);
        assert_eq!(written.crc32(), original.0);
        assert_eq!(written.compressed_size(), original.1);
        assert_eq!(written.size(), original.2);
        assert_eq!(written.last_modified(), Some(deterministic_timestamp()));
        assert_eq!(compressed_bytes(&first, index), original.3.as_slice());
    }
}

#[cfg(test)]
#[test]
fn preserve_mode_keeps_unknown_entry_bytes() {
    use crate::{
        opc::package::Package,
        zip::{limits::OpenOptions, reader::from_bytes},
    };

    let package_bytes = include_bytes!("../../../../fixtures/minimal.pptx");
    let entries = from_bytes(package_bytes).expect("minimal fixture reads");
    let package =
        Package::from_zip_entries(&entries, &OpenOptions::default()).expect("package loads");

    let written = write_package_vec(&package, &WriteOptions::default())
        .expect("package writes in preserve mode");
    let written_entries = from_bytes(&written).expect("written package reads");

    let original_by_name = entries
        .iter()
        .map(|entry| (entry.meta.original_name.as_str(), entry.bytes.as_slice()))
        .collect::<BTreeMap<_, _>>();
    let written_by_name = written_entries
        .iter()
        .map(|entry| (entry.meta.original_name.as_str(), entry.bytes.as_slice()))
        .collect::<BTreeMap<_, _>>();

    assert_eq!(
        written_by_name.keys().collect::<Vec<_>>(),
        original_by_name.keys().collect::<Vec<_>>()
    );
    for (name, original_bytes) in original_by_name {
        assert_eq!(
            written_by_name.get(name).copied(),
            Some(original_bytes),
            "entry {name} changed"
        );
    }
}

#[cfg(test)]
#[test]
fn no_edit_write_preserves_clean_malformed_xml_part() {
    use crate::{opc::package::Package, zip::reader::from_bytes};

    let malformed_bytes = br#"<root><unclosed></root>"#;
    let mut package = Package::new();
    package
        .insert_zip_entry("customXml/item1.xml", malformed_bytes.to_vec())
        .expect("custom XML inserted");
    package
        .content_types_mut()
        .insert_default("xml", "application/xml");

    let written = write_package_vec(&package, &WriteOptions::default())
        .expect("no-edit write raw-copies clean malformed XML");
    let written_entries = from_bytes(&written).expect("written package reads");
    let written_part = written_entries
        .iter()
        .find(|entry| entry.meta.original_name == "customXml/item1.xml")
        .expect("custom XML part preserved");

    assert_eq!(written_part.bytes, malformed_bytes);
}

#[cfg(test)]
fn compressed_bytes(package: &[u8], index: usize) -> &[u8] {
    use std::io::Cursor;

    let mut archive = zip::ZipArchive::new(Cursor::new(package)).expect("package opens");
    let file = archive.by_index(index).expect("entry exists");
    let start = file.data_start().expect("entry data start exists") as usize;
    let end = start + file.compressed_size() as usize;
    &package[start..end]
}

#[cfg(test)]
fn write_vec_with_options(
    source_package: &[u8],
    entries: &[WriteEntry<'_>],
    options: &WriteOptions,
) -> Result<Vec<u8>> {
    let mut output = Cursor::new(Vec::new());
    write_writer_with_options(source_package, entries, &mut output, options)?;
    Ok(output.into_inner())
}
