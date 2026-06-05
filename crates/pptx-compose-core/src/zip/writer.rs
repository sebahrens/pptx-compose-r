use std::io::{Cursor, Read, Seek, Write};

use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::{
    error::{Error, ErrorCode, Result},
    zip::{ZipEntryMetadata, reader::RawEntry},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriteOptions {
    pub mode: WriteMode,
    pub compression_method: zip::CompressionMethod,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            mode: WriteMode::Preserve,
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

    let ordered_entries = ordered_entries(entries, source.len(), options.mode);
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
            WriteMode::Preserve => self.options.compression_method,
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

fn ordered_entries<'a>(
    entries: &'a [WriteEntry<'a>],
    source_entry_count: usize,
    mode: WriteMode,
) -> Vec<&'a WriteEntry<'a>> {
    let mut ordered_entries: Vec<_> = entries.iter().collect();
    if mode == WriteMode::Deterministic {
        ordered_entries.sort_by(|left, right| {
            deterministic_order_key(left, source_entry_count)
                .cmp(&deterministic_order_key(right, source_entry_count))
        });
    }
    ordered_entries
}

fn deterministic_order_key<'a>(
    entry: &'a WriteEntry<'_>,
    source_entry_count: usize,
) -> (u8, usize, &'a str) {
    let meta = entry.meta();
    if meta.entry_index < source_entry_count {
        (0, meta.entry_index, "")
    } else {
        (1, usize::MAX, entry.name())
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
fn deterministic_is_stable_cross_run() {
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
        assert_eq!(written.last_modified(), Some(deterministic_timestamp()));
        assert_eq!(
            compressed_bytes(&first, index),
            compressed_bytes(package, index)
        );
    }
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
