use std::{
    collections::HashSet,
    io::{Cursor, Read, Seek, SeekFrom},
};

use zip::ZipArchive;

use crate::{
    error::{Error, Result},
    opc::part_name::{PartName, reject_unsafe_entry},
    zip::{
        ZipEntryMetadata,
        limits::{
            LimitEnforcingReader, OpenOptions, ensure_compressed_package_size, ensure_part_count,
        },
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawEntry {
    pub name: PartName,
    pub bytes: Vec<u8>,
    pub meta: ZipEntryMetadata,
}

pub fn from_bytes(bytes: &[u8]) -> Result<Vec<RawEntry>> {
    from_bytes_with_options(bytes, &OpenOptions::default())
}

pub fn from_bytes_with_options(bytes: &[u8], options: &OpenOptions) -> Result<Vec<RawEntry>> {
    ensure_compressed_package_size(bytes.len() as u64, &options.resource_limits)?;
    open_reader_with_options(Cursor::new(bytes), options)
}

pub fn open_reader<R>(reader: R) -> Result<Vec<RawEntry>>
where
    R: Read + Seek,
{
    open_reader_with_options(reader, &OpenOptions::default())
}

pub fn open_reader_with_options<R>(reader: R, options: &OpenOptions) -> Result<Vec<RawEntry>>
where
    R: Read + Seek,
{
    read_entries_with_options(reader, options)
}

pub fn read_entries<R>(reader: R) -> Result<Vec<RawEntry>>
where
    R: Read + Seek,
{
    read_entries_with_options(reader, &OpenOptions::default())
}

pub fn read_entries_with_options<R>(mut reader: R, options: &OpenOptions) -> Result<Vec<RawEntry>>
where
    R: Read + Seek,
{
    let compressed_package_bytes = stream_len(&mut reader)?;
    ensure_compressed_package_size(compressed_package_bytes, &options.resource_limits)?;

    let mut archive = ZipArchive::new(reader)
        .map_err(|source| Error::parse_error("Could not open ZIP package.", source))?;
    read_archive_entries(&mut archive, options)
}

fn read_archive_entries<R>(
    archive: &mut ZipArchive<R>,
    options: &OpenOptions,
) -> Result<Vec<RawEntry>>
where
    R: Read + Seek,
{
    let mut entries = Vec::with_capacity(archive.len());
    let mut names = HashSet::with_capacity(archive.len());
    let mut package_uncompressed_bytes = 0;

    for index in 0..archive.len() {
        ensure_part_count(index + 1, &options.resource_limits)?;
        let mut entry = archive.by_index(index).map_err(|source| {
            Error::parse_error("Could not read ZIP entry metadata and bytes.", source)
        })?;
        let entry_name = entry.name().to_owned();

        reject_unsafe_entry(&entry_name)?;
        let normalized_name = normalize_entry_name(&entry_name, entry.is_dir())?;
        if !names.insert(normalized_name.clone()) {
            return Err(Error::duplicate_part(format!(
                "Package contains more than one ZIP entry normalized to {normalized_name}."
            )));
        }

        let meta = ZipEntryMetadata {
            original_name: entry_name,
            compression_method: entry.compression(),
            crc32: entry.crc32(),
            compressed_size: entry.compressed_size(),
            uncompressed_size: entry.size(),
            last_modified: entry.last_modified(),
            external_attrs: entry.unix_mode(),
            is_dir: entry.is_dir(),
        };

        let mut bytes = Vec::new();
        let mut enforcing_reader = LimitEnforcingReader::new(
            &mut entry,
            &options.resource_limits,
            &meta.original_name,
            meta.compressed_size,
            &mut package_uncompressed_bytes,
        );
        if let Err(source) = enforcing_reader.read_to_end(&mut bytes) {
            if let Some(error) = enforcing_reader.take_error() {
                return Err(error);
            }
            return Err(Error::parse_error(
                "Could not read ZIP entry bytes.",
                source,
            ));
        }
        entries.push(RawEntry {
            name: normalized_name,
            bytes,
            meta,
        });
    }

    Ok(entries)
}

fn stream_len<R>(reader: &mut R) -> Result<u64>
where
    R: Seek,
{
    let current = reader
        .stream_position()
        .map_err(|source| Error::parse_error("Could not read ZIP stream position.", source))?;
    let end = reader
        .seek(SeekFrom::End(0))
        .map_err(|source| Error::parse_error("Could not read ZIP stream length.", source))?;
    reader
        .seek(SeekFrom::Start(current))
        .map_err(|source| Error::parse_error("Could not restore ZIP stream position.", source))?;
    Ok(end)
}

fn normalize_entry_name(entry_name: &str, is_dir: bool) -> Result<PartName> {
    if is_dir {
        PartName::from_zip_entry(entry_name.trim_end_matches(['/', '\\']))
    } else {
        PartName::from_zip_entry(entry_name)
    }
}

#[cfg(test)]
#[test]
fn reads_minimal_pptx() {
    let package = include_bytes!("../../../../fixtures/minimal.pptx");

    let entries = from_bytes(package).expect("minimal fixture reads");

    let actual: Vec<(&str, usize)> = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.bytes.len()))
        .collect();
    assert_eq!(
        actual,
        [
            ("/[Content_Types].xml", 564),
            ("/_rels/.rels", 300),
            ("/ppt/presentation.xml", 382),
            ("/ppt/_rels/presentation.xml.rels", 288),
            ("/ppt/slides/slide1.xml", 445),
        ]
    );

    for entry in entries {
        assert!(!entry.meta.original_name.starts_with('/'));
        assert_eq!(entry.bytes.len() as u64, entry.meta.uncompressed_size);
        assert!(entry.meta.compressed_size > 0);
        assert!(!entry.meta.is_dir);
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    use super::from_bytes;
    use crate::error::ErrorCode;

    #[test]
    fn preserves_archive_order_and_metadata() {
        let package = zip_with_entries([
            ("b.xml", b"second".as_slice()),
            ("a.xml", b"first".as_slice()),
        ]);

        let entries = from_bytes(&package).expect("ZIP reads");

        assert_eq!(entries[0].name.as_str(), "/b.xml");
        assert_eq!(entries[0].bytes, b"second");
        assert_eq!(entries[0].meta.original_name, "b.xml");
        assert_eq!(
            entries[0].meta.compression_method,
            CompressionMethod::Stored
        );
        assert_eq!(entries[0].meta.uncompressed_size, 6);
        assert_eq!(entries[1].name.as_str(), "/a.xml");
        assert_eq!(entries[1].bytes, b"first");
    }

    #[test]
    fn retains_directory_entries_as_metadata() {
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut bytes);
            let options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            writer
                .add_directory("ppt/", options)
                .expect("add directory");
            writer
                .start_file("ppt/presentation.xml", options)
                .expect("start file");
            writer.write_all(b"<p/>").expect("write file");
            writer.finish().expect("finish ZIP package");
        }

        let entries = from_bytes(&bytes.into_inner()).expect("ZIP reads");

        assert_eq!(entries[0].name.as_str(), "/ppt");
        assert!(entries[0].meta.is_dir);
        assert!(entries[0].bytes.is_empty());
        assert_eq!(entries[1].name.as_str(), "/ppt/presentation.xml");
    }

    #[test]
    fn rejects_traversal() {
        for entry_name in ["../../etc/passwd", "%2e%2e/%2e%2e/etc/passwd"] {
            let package = zip_with_entries([(entry_name, b"test".as_slice())]);

            let error = from_bytes(&package).expect_err("unsafe ZIP entry must reject package");

            assert_eq!(error.code(), ErrorCode::UnsafePath);
        }
    }

    fn zip_with_entries<const N: usize>(entries: [(&str, &[u8]); N]) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut bytes);
            let options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

            for (name, contents) in entries {
                writer.start_file(name, options).expect("start ZIP entry");
                writer.write_all(contents).expect("write ZIP entry");
            }
            writer.finish().expect("finish ZIP package");
        }

        bytes.into_inner()
    }
}
