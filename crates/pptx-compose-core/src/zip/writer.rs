use std::io::{Cursor, Read, Seek, Write};

use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::{
    error::{Error, ErrorCode, Result},
    zip::{ZipEntryMetadata, reader::RawEntry},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriteOptions {
    pub compression_method: zip::CompressionMethod,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            compression_method: zip::CompressionMethod::Deflated,
        }
    }
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

    for entry in entries {
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

        self.writer.raw_copy_file(source_file).map_err(|source| {
            Error::with_source(
                ErrorCode::WriteFailed,
                "Could not raw-copy clean ZIP entry.",
                source,
            )
        })
    }

    pub fn write_dirty(&mut self, name: &str, bytes: &[u8], meta: &ZipEntryMetadata) -> Result<()> {
        let mut file_options = SimpleFileOptions::default()
            .compression_method(self.options.compression_method)
            .large_file(bytes.len() > u32::MAX as usize);
        if let Some(last_modified) = meta.last_modified.filter(zip::DateTime::is_valid) {
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
fn compressed_bytes(package: &[u8], index: usize) -> &[u8] {
    use std::io::Cursor;

    let mut archive = zip::ZipArchive::new(Cursor::new(package)).expect("package opens");
    let file = archive.by_index(index).expect("entry exists");
    let start = file.data_start().expect("entry data start exists") as usize;
    let end = start + file.compressed_size() as usize;
    &package[start..end]
}
