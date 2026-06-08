use crate::{
    error::{Error, Result},
    zip::reader::RawEntry,
};

const CFBF_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const ZIP_LOCAL_FILE_HEADER_MAGIC: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];
const ENCRYPTION_INFO_ENTRY: &str = "EncryptionInfo";
const ENCRYPTED_PACKAGE_ENTRY: &str = "EncryptedPackage";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageFormat {
    ZipCandidate,
    OleOrEncrypted,
    Unknown,
}

#[must_use]
pub fn sniff_package_format(bytes: &[u8]) -> PackageFormat {
    if bytes.starts_with(&CFBF_MAGIC) {
        PackageFormat::OleOrEncrypted
    } else if bytes.starts_with(&ZIP_LOCAL_FILE_HEADER_MAGIC) {
        PackageFormat::ZipCandidate
    } else {
        PackageFormat::Unknown
    }
}

pub fn reject_unsupported_package_format(bytes: &[u8]) -> Result<()> {
    match sniff_package_format(bytes) {
        PackageFormat::ZipCandidate => Ok(()),
        PackageFormat::OleOrEncrypted => Err(Error::unsupported_package(
            "The input is an OLE/CFBF compound file, which may be an encrypted OOXML deck or a legacy binary PowerPoint file. This version only opens unencrypted PPTX ZIP packages.",
        )),
        PackageFormat::Unknown => Err(Error::unsupported_package(
            "The input is not a recognized PPTX ZIP package.",
        )),
    }
}

pub fn reject_encrypted_zip_entries(entries: &[RawEntry]) -> Result<()> {
    for entry in entries {
        if is_encryption_marker(&entry.meta.original_name) {
            return Err(Error::unsupported_package(
                "The PPTX package contains OOXML encryption streams. This version does not decrypt encrypted decks.",
            ));
        }
    }

    Ok(())
}

fn is_encryption_marker(entry_name: &str) -> bool {
    entry_name == ENCRYPTION_INFO_ENTRY || entry_name == ENCRYPTED_PACKAGE_ENTRY
}

#[cfg(test)]
#[test]
fn rejects_cfbf_before_zip_parse() {
    let package = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0x00];

    let error = reject_unsupported_package_format(&package).expect_err("CFBF must be rejected");

    assert_eq!(error.code(), crate::error::ErrorCode::UnsupportedPackage);
}

#[cfg(test)]
mod tests {
    use super::{
        PackageFormat, reject_encrypted_zip_entries, reject_unsupported_package_format,
        sniff_package_format,
    };
    use crate::{
        error::ErrorCode,
        opc::part_name::PartName,
        zip::{ZipEntryMetadata, reader::RawEntry},
    };

    #[test]
    fn detects_package_formats_from_leading_bytes() {
        assert_eq!(
            sniff_package_format(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0x00]),
            PackageFormat::OleOrEncrypted
        );
        assert_eq!(
            sniff_package_format(&[0x50, 0x4B, 0x03, 0x04, 0x14, 0x00]),
            PackageFormat::ZipCandidate
        );
        assert_eq!(sniff_package_format(b"not a pptx"), PackageFormat::Unknown);
    }

    #[test]
    fn rejects_zip_with_encrypted_package_entry_as_unsupported_package() {
        let entries = raw_entries(["EncryptedPackage"]);

        let error = reject_encrypted_zip_entries(&entries)
            .expect_err("encrypted OOXML ZIP must be rejected");

        assert_eq!(error.code(), ErrorCode::UnsupportedPackage);
    }

    #[test]
    fn accepts_zip_without_encryption_entries() {
        let entries = raw_entries(["[Content_Types].xml", "ppt/presentation.xml"]);

        reject_encrypted_zip_entries(&entries).expect("ordinary ZIP package should pass sniff");
    }

    #[test]
    fn rejects_non_zip_input_as_unsupported_package() {
        let error = reject_unsupported_package_format(b"not a pptx")
            .expect_err("non-ZIP input must be rejected");

        assert_eq!(error.code(), ErrorCode::UnsupportedPackage);
    }

    fn raw_entries<const N: usize>(names: [&str; N]) -> Vec<RawEntry> {
        names
            .into_iter()
            .enumerate()
            .map(|(index, name)| RawEntry {
                name: PartName::from_zip_entry(name).expect("valid test part name"),
                bytes: Vec::new(),
                meta: ZipEntryMetadata {
                    entry_index: index,
                    original_name: name.to_owned(),
                    compression_method: zip::CompressionMethod::Stored,
                    crc32: 0,
                    compressed_size: 0,
                    uncompressed_size: 0,
                    last_modified: None,
                    external_attrs: None,
                    is_dir: false,
                },
            })
            .collect()
    }
}
