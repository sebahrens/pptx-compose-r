use zip::{CompressionMethod, DateTime};

pub mod limits;
pub mod reader;
pub mod sniff;
pub mod writer;

pub use reader::{RawEntry, ZipEntry, read_zip_entries};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZipEntryMetadata {
    pub entry_index: usize,
    pub original_name: String,
    pub compression_method: CompressionMethod,
    pub crc32: u32,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub last_modified: Option<DateTime>,
    pub external_attrs: Option<u32>,
    pub is_dir: bool,
}
