use std::io::{Cursor, Read, Seek};

use zip::ZipArchive;

use crate::{
    error::{Error, Result},
    opc::{part::PartStore, part_name::reject_unsafe_entry},
};

pub fn from_bytes(bytes: &[u8]) -> Result<PartStore> {
    open_reader(Cursor::new(bytes))
}

pub fn open_reader<R>(reader: R) -> Result<PartStore>
where
    R: Read + Seek,
{
    let mut archive = ZipArchive::new(reader)
        .map_err(|source| Error::parse_error("Could not open ZIP package.", source))?;
    read_entries(&mut archive)
}

pub fn read_entries<R>(archive: &mut ZipArchive<R>) -> Result<PartStore>
where
    R: Read + Seek,
{
    let mut store = PartStore::new();

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index_raw(index)
            .map_err(|source| Error::parse_error("Could not read ZIP entry metadata.", source))?;
        let entry_name = entry.name().to_owned();

        reject_unsafe_entry(&entry_name)?;

        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|source| Error::parse_error("Could not read ZIP entry bytes.", source))?;
        store.insert_zip_entry(entry_name, bytes)?;
    }

    Ok(store)
}

#[cfg(test)]
#[test]
fn rejects_traversal() {
    use crate::error::ErrorCode;

    for entry_name in ["../../etc/passwd", "%2e%2e/%2e%2e/etc/passwd"] {
        let package = zip_with_entry(entry_name);

        let error = from_bytes(&package).expect_err("unsafe ZIP entry must reject package");

        assert_eq!(error.code(), ErrorCode::UnsafePath);
    }
}

#[cfg(test)]
fn zip_with_entry(name: &str) -> Vec<u8> {
    use std::io::{Cursor, Write};

    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    let mut bytes = Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut bytes);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

        writer.start_file(name, options).expect("start ZIP entry");
        writer.write_all(b"test").expect("write ZIP entry");
        writer.finish().expect("finish ZIP package");
    }

    bytes.into_inner()
}
