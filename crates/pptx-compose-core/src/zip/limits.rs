use std::io::{self, Read};

use crate::error::{Error, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceLimits {
    pub max_compressed_package_bytes: u64,
    pub max_uncompressed_package_bytes: u64,
    pub max_part_count: usize,
    pub max_single_part_uncompressed_bytes: u64,
    pub max_media_part_bytes: u64,
    pub max_per_entry_compression_ratio: u64,
    pub max_xml_depth: usize,
    pub max_xml_node_count: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_compressed_package_bytes: 524_288_000,
            max_uncompressed_package_bytes: 2_147_483_648,
            max_part_count: 10_000,
            max_single_part_uncompressed_bytes: 268_435_456,
            max_media_part_bytes: 67_108_864,
            max_per_entry_compression_ratio: 200,
            max_xml_depth: 256,
            max_xml_node_count: 5_000_000,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OpenOptions {
    pub resource_limits: ResourceLimits,
}

pub(crate) struct LimitEnforcingReader<'a, R> {
    inner: R,
    limits: &'a ResourceLimits,
    entry_name: &'a str,
    entry_specific_uncompressed_limit: Option<u64>,
    compressed_size: u64,
    entry_uncompressed_bytes: u64,
    package_uncompressed_bytes: &'a mut u64,
    error: Option<Error>,
}

impl<'a, R> LimitEnforcingReader<'a, R>
where
    R: Read,
{
    pub(crate) fn new(
        inner: R,
        limits: &'a ResourceLimits,
        entry_name: &'a str,
        entry_specific_uncompressed_limit: Option<u64>,
        compressed_size: u64,
        package_uncompressed_bytes: &'a mut u64,
    ) -> Self {
        Self {
            inner,
            limits,
            entry_name,
            entry_specific_uncompressed_limit,
            compressed_size,
            entry_uncompressed_bytes: 0,
            package_uncompressed_bytes,
            error: None,
        }
    }

    #[cfg(test)]
    const fn uncompressed_read(&self) -> u64 {
        self.entry_uncompressed_bytes
    }

    pub(crate) fn take_error(&mut self) -> Option<Error> {
        self.error.take()
    }

    fn max_read_len(&self, requested: usize) -> usize {
        let single_part = bytes_until_crossing(
            self.entry_uncompressed_bytes,
            self.limits.max_single_part_uncompressed_bytes,
        );
        let entry_specific = self
            .entry_specific_uncompressed_limit
            .map(|limit| bytes_until_crossing(self.entry_uncompressed_bytes, limit))
            .unwrap_or(u64::MAX);
        let package = bytes_until_crossing(
            *self.package_uncompressed_bytes,
            self.limits.max_uncompressed_package_bytes,
        );
        let ratio = if self.compressed_size == 0 {
            if self.entry_uncompressed_bytes == 0 {
                1
            } else {
                0
            }
        } else {
            bytes_until_crossing(
                self.entry_uncompressed_bytes,
                self.compressed_size
                    .saturating_mul(self.limits.max_per_entry_compression_ratio),
            )
        };

        let limit = single_part.min(entry_specific).min(package).min(ratio);
        requested.min(usize::try_from(limit).unwrap_or(usize::MAX))
    }

    fn check_limits(&self) -> Result<()> {
        if self.entry_uncompressed_bytes > self.limits.max_single_part_uncompressed_bytes {
            return Err(Error::resource_limit_exceeded(format!(
                "ZIP entry {} exceeded the maximum single-part uncompressed size of {} bytes.",
                self.entry_name, self.limits.max_single_part_uncompressed_bytes
            )));
        }

        if let Some(limit) = self.entry_specific_uncompressed_limit
            && self.entry_uncompressed_bytes > limit
        {
            return Err(Error::resource_limit_exceeded(format!(
                "ZIP entry {} exceeded the maximum media part size of {} bytes.",
                self.entry_name, limit
            )));
        }

        if *self.package_uncompressed_bytes > self.limits.max_uncompressed_package_bytes {
            return Err(Error::resource_limit_exceeded(format!(
                "ZIP package exceeded the maximum uncompressed size of {} bytes.",
                self.limits.max_uncompressed_package_bytes
            )));
        }

        if self.compressed_size == 0 {
            if self.entry_uncompressed_bytes > 0 {
                return Err(Error::resource_limit_exceeded(format!(
                    "ZIP entry {} has non-empty uncompressed data with zero compressed bytes.",
                    self.entry_name
                )));
            }
            return Ok(());
        }

        let max_uncompressed_for_ratio = self
            .compressed_size
            .saturating_mul(self.limits.max_per_entry_compression_ratio);
        if self.entry_uncompressed_bytes > max_uncompressed_for_ratio {
            return Err(Error::resource_limit_exceeded(format!(
                "ZIP entry {} exceeded the maximum compression ratio of {}:1.",
                self.entry_name, self.limits.max_per_entry_compression_ratio
            )));
        }

        Ok(())
    }
}

impl<R> Read for LimitEnforcingReader<'_, R>
where
    R: Read,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        if self.error.is_some() {
            return Err(limit_io_error());
        }

        let read_len = self.max_read_len(buf.len()).max(1);
        let count = self.inner.read(&mut buf[..read_len])?;
        self.entry_uncompressed_bytes = self
            .entry_uncompressed_bytes
            .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
        *self.package_uncompressed_bytes = (*self.package_uncompressed_bytes)
            .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));

        if let Err(error) = self.check_limits() {
            self.error = Some(error);
            return Err(limit_io_error());
        }

        Ok(count)
    }
}

pub(crate) fn ensure_compressed_package_size(
    compressed_package_bytes: u64,
    limits: &ResourceLimits,
) -> Result<()> {
    if compressed_package_bytes > limits.max_compressed_package_bytes {
        return Err(Error::resource_limit_exceeded(format!(
            "ZIP package exceeded the maximum compressed size of {} bytes.",
            limits.max_compressed_package_bytes
        )));
    }

    Ok(())
}

pub(crate) fn ensure_part_count(part_count: usize, limits: &ResourceLimits) -> Result<()> {
    if part_count > limits.max_part_count {
        return Err(Error::resource_limit_exceeded(format!(
            "ZIP package exceeded the maximum part count of {}.",
            limits.max_part_count
        )));
    }

    Ok(())
}

fn bytes_until_crossing(current: u64, allowed: u64) -> u64 {
    allowed.saturating_sub(current).saturating_add(1)
}

fn limit_io_error() -> io::Error {
    io::Error::other("ZIP resource limit exceeded")
}

#[cfg(test)]
#[test]
fn aborts_zip_bomb_during_inflate() {
    use std::io::{Cursor, Read};

    use zip::ZipArchive;

    use crate::error::ErrorCode;

    const EXPANDED_SIZE: usize = 1_000_000;

    let package = zip_with_entry("ppt/slides/slide1.xml", &vec![b'a'; EXPANDED_SIZE]);
    let mut archive = ZipArchive::new(Cursor::new(package)).expect("open ZIP");
    let mut entry = archive.by_name("ppt/slides/slide1.xml").expect("entry");
    let compressed_size = entry.compressed_size();
    assert!(
        compressed_size > 0,
        "test archive must have measurable compressed bytes"
    );

    let limits = ResourceLimits {
        max_single_part_uncompressed_bytes: u64::MAX,
        max_uncompressed_package_bytes: u64::MAX,
        ..ResourceLimits::default()
    };
    let mut package_uncompressed = 0;
    let mut reader = LimitEnforcingReader::new(
        &mut entry,
        &limits,
        "ppt/slides/slide1.xml",
        None,
        compressed_size,
        &mut package_uncompressed,
    );
    let mut bytes = Vec::new();

    let read_error = reader
        .read_to_end(&mut bytes)
        .expect_err("ratio limit must abort");
    let inflated_before_abort = reader.uncompressed_read();
    let error = reader
        .take_error()
        .expect("resource-limit error is preserved");

    assert_eq!(error.code(), ErrorCode::ResourceLimitExceeded);
    assert_eq!(read_error.kind(), std::io::ErrorKind::Other);
    assert!(inflated_before_abort < EXPANDED_SIZE as u64);
    assert!(
        inflated_before_abort <= compressed_size * limits.max_per_entry_compression_ratio + 1,
        "inflate should stop close to the 200:1 crossing, got {inflated_before_abort} bytes for {compressed_size} compressed bytes"
    );
}

#[cfg(test)]
fn zip_with_entry(name: &str, contents: &[u8]) -> Vec<u8> {
    use std::io::{Cursor, Write};

    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    let mut bytes = Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut bytes);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        writer.start_file(name, options).expect("start ZIP entry");
        writer.write_all(contents).expect("write ZIP entry");
        writer.finish().expect("finish ZIP package");
    }

    bytes.into_inner()
}
