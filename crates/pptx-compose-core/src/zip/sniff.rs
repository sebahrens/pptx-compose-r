use std::io::{Read, Seek, SeekFrom};

use zip::ZipArchive;

use crate::error::{Error, Result};

const CFBF_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const ZIP_LOCAL_FILE_HEADER_MAGIC: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];
const ENCRYPTION_INFO_ENTRY: &str = "EncryptionInfo";
const ENCRYPTED_PACKAGE_ENTRY: &str = "EncryptedPackage";

pub fn sniff_package<R>(reader: &mut R) -> Result<()>
where
    R: Read + Seek,
{
    let original_position = reader.stream_position()?;
    reader.seek(SeekFrom::Start(0))?;

    let result = sniff_package_from_start(reader);
    let restore_result = reader.seek(SeekFrom::Start(original_position));

    match (result, restore_result) {
        (Ok(()), Ok(_)) => Ok(()),
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(Error::from(error)),
    }
}

fn sniff_package_from_start<R>(reader: &mut R) -> Result<()>
where
    R: Read + Seek,
{
    let mut head = [0_u8; CFBF_MAGIC.len()];
    let bytes_read = reader.read(&mut head)?;

    if bytes_read >= CFBF_MAGIC.len() && head == CFBF_MAGIC {
        return Err(Error::unsupported_package(
            "The input is an OLE/CFBF compound file, which may be an encrypted OOXML deck or a legacy binary PowerPoint file. This version only opens unencrypted PPTX ZIP packages.",
        ));
    }

    if bytes_read < ZIP_LOCAL_FILE_HEADER_MAGIC.len()
        || head[..ZIP_LOCAL_FILE_HEADER_MAGIC.len()] != ZIP_LOCAL_FILE_HEADER_MAGIC
    {
        return Err(Error::unsupported_package(
            "The input is not a recognized PPTX ZIP package.",
        ));
    }

    reader.seek(SeekFrom::Start(0))?;
    let mut archive = ZipArchive::new(reader).map_err(|source| {
        Error::parse_error(
            "The input starts like a ZIP package but could not be opened as a ZIP archive.",
            source,
        )
    })?;

    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index).map_err(|source| {
            Error::parse_error(
                "Could not inspect ZIP entries for encryption markers.",
                source,
            )
        })?;
        let name = entry.name();

        if name == ENCRYPTION_INFO_ENTRY || name == ENCRYPTED_PACKAGE_ENTRY {
            return Err(Error::unsupported_package(
                "The PPTX package contains OOXML encryption streams. This version does not decrypt encrypted decks.",
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    use super::sniff_package;
    use crate::error::ErrorCode;

    #[test]
    fn rejects_cfbf_magic_as_unsupported_package() {
        let mut package = Cursor::new([0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0x00]);

        let error = sniff_package(&mut package).expect_err("CFBF must be rejected");

        assert_eq!(error.code(), ErrorCode::UnsupportedPackage);
    }

    #[test]
    fn rejects_zip_with_encrypted_package_entry_as_unsupported_package() {
        let bytes = zip_with_entries(["EncryptedPackage"]);
        let mut package = Cursor::new(bytes);

        let error = sniff_package(&mut package).expect_err("encrypted OOXML ZIP must be rejected");

        assert_eq!(error.code(), ErrorCode::UnsupportedPackage);
    }

    #[test]
    fn accepts_zip_without_encryption_entries_and_preserves_reader_position() {
        let bytes = zip_with_entries(["[Content_Types].xml", "ppt/presentation.xml"]);
        let mut package = Cursor::new(bytes);
        package.set_position(2);

        sniff_package(&mut package).expect("ordinary ZIP package should pass sniff");

        assert_eq!(package.position(), 2);
    }

    #[test]
    fn rejects_non_zip_input_as_unsupported_package() {
        let mut package = Cursor::new(*b"not a pptx");

        let error = sniff_package(&mut package).expect_err("non-ZIP input must be rejected");

        assert_eq!(error.code(), ErrorCode::UnsupportedPackage);
    }

    fn zip_with_entries<const N: usize>(names: [&str; N]) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut bytes);
            let options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

            for name in names {
                writer
                    .start_file(name, options)
                    .expect("start test ZIP entry");
                writer.write_all(b"test").expect("write test ZIP entry");
            }

            writer.finish().expect("finish test ZIP");
        }

        bytes.into_inner()
    }
}
