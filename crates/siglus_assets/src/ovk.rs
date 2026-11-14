//! OVK (Ogg/Vorbis pack) and OWP (XORed Ogg) helpers.
//!
//! Helpers for OVK (Ogg/Vorbis pack) and OWP (XORed Ogg) audio formats.

use crate::ogg_xor::{BoundedFile, validate_subrange};
use crate::vorbis;
use anyhow::{Result, bail, ensure};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy)]
pub struct OvkEntry {
    pub size: u32,
    pub offset: u32,
    pub no: u32,
    pub sample_count: u32,
}

/// Validated metadata for a pack backed by an immutable range reader.
#[derive(Debug, Clone)]
pub struct OvkIndex {
    entries: Vec<OvkEntry>,
}

impl OvkIndex {
    pub fn read(
        file_len: u64,
        mut read: impl FnMut(u64, usize) -> Result<Vec<u8>>,
    ) -> Result<Self> {
        ensure!(file_len >= 4, "OVK: header too small");
        let head = read(0, 4)?;
        ensure!(head.len() == 4, "OVK: header truncated");
        let count = u32::from_le_bytes(head[..].try_into().unwrap()) as usize;
        ensure!(count != 0, "OVK: zero entries");
        let table_len = count
            .checked_mul(16)
            .ok_or_else(|| anyhow::anyhow!("OVK: index too large"))?;
        ensure!(
            table_len as u64 <= file_len - 4,
            "OVK: entry table truncated"
        );
        let bytes = read(4, table_len)?;
        ensure!(bytes.len() == table_len, "OVK: entry table truncated");
        let mut entries = Vec::new();
        entries.try_reserve_exact(count)?;
        for (i, row) in bytes.chunks_exact(16).enumerate() {
            let entry = OvkEntry {
                size: u32::from_le_bytes(row[..4].try_into().unwrap()),
                offset: u32::from_le_bytes(row[4..8].try_into().unwrap()),
                no: u32::from_le_bytes(row[8..12].try_into().unwrap()),
                sample_count: u32::from_le_bytes(row[12..16].try_into().unwrap()),
            };
            if entry.size != 0 {
                validate_subrange(file_len, u64::from(entry.offset), u64::from(entry.size))
                    .map_err(|err| anyhow::anyhow!("OVK entry[{i}] out of range: {err}"))?;
            }
            entries.push(entry);
        }
        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[OvkEntry] {
        &self.entries
    }

    pub fn extract_entry(
        &self,
        idx: usize,
        mut read: impl FnMut(u64, usize) -> Result<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        let entry = self
            .entries
            .get(idx)
            .ok_or_else(|| anyhow::anyhow!("OVK: entry index out of range: {idx}"))?;
        ensure!(entry.size != 0, "OVK: entry[{idx}] has zero size");
        let bytes = read(u64::from(entry.offset), entry.size as usize)?;
        ensure!(
            bytes.len() == entry.size as usize,
            "OVK: entry[{idx}] truncated"
        );
        Ok(bytes)
    }
}

#[cfg(test)]
mod range_tests {
    use super::*;

    #[test]
    fn large_pack_reads_only_index_and_selected_voice() {
        let file_len = 1024 * 1024 * 1024u64;
        let target_offset = file_len - 100;
        let mut header = 3u32.to_le_bytes().to_vec();
        // The selected entry is neither the first row nor ordered by number.
        for fields in [
            [12, 4096, 90, 100],
            [7, target_offset as u32, 17, 200],
            [20, 8192, 3, 300],
        ] {
            for field in fields {
                header.extend_from_slice(&field.to_le_bytes());
            }
        }
        let payload = b"voice-B";
        let mut requested_bytes = 0;
        let mut read = |offset: u64, length: usize| -> Result<Vec<u8>> {
            requested_bytes += length;
            if offset + length as u64 <= header.len() as u64 {
                return Ok(header[offset as usize..offset as usize + length].to_vec());
            }
            if offset == target_offset && length == payload.len() {
                return Ok(payload.to_vec());
            }
            bail!("unrelated OVK bytes requested: offset={offset} length={length}");
        };
        let index = OvkIndex::read(file_len, &mut read).unwrap();
        assert_eq!(index.entries().len(), 3);
        let idx = index
            .entries()
            .iter()
            .position(|entry| entry.no == 17)
            .unwrap();
        assert_eq!(index.extract_entry(idx, &mut read).unwrap(), payload);
        assert_eq!(index.extract_entry(idx, &mut read).unwrap(), payload);
        assert!(
            requested_bytes < 1024,
            "single-voice reads must stay bounded"
        );
    }

    #[test]
    fn malformed_index_fails_before_reading_audio() {
        for (file_len, count) in [(3, 1), (100, 0), (100, u32::MAX), (19, 1)] {
            let result = OvkIndex::read(file_len, |offset, length| {
                assert_eq!((offset, length), (0, 4));
                Ok(count.to_le_bytes().to_vec())
            });
            assert!(result.is_err());
        }
        assert!(OvkIndex::read(100, |_, _| Ok(vec![0; 3])).is_err());
        assert!(
            OvkIndex::read(100, |offset, _| Ok(if offset == 0 {
                1u32.to_le_bytes().to_vec()
            } else {
                vec![0; 15]
            }))
            .is_err()
        );
    }

    fn index_with_entry(size: u32, offset: u32) -> Result<OvkIndex> {
        OvkIndex::read(1024, |position, _| {
            Ok(if position == 0 {
                1u32.to_le_bytes().to_vec()
            } else {
                [size, offset, 17, 42]
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect()
            })
        })
    }

    #[test]
    fn invalid_entries_and_short_payloads_fail_closed() {
        assert!(index_with_entry(30, 1000).is_err());
        assert!(index_with_entry(30, u32::MAX).is_err());
        let empty = index_with_entry(0, u32::MAX).unwrap();
        assert!(
            empty
                .extract_entry(0, |_, _| panic!("empty entry must not read"))
                .is_err()
        );
        let index = index_with_entry(3, 100).unwrap();
        assert!(
            index
                .extract_entry(1, |_, _| panic!("invalid index must not read"))
                .is_err()
        );
        assert!(index.extract_entry(0, |_, _| Ok(vec![1, 2])).is_err());
        assert!(
            index
                .extract_entry(0, |_, _| bail!("source failed"))
                .unwrap_err()
                .to_string()
                .contains("source failed")
        );
    }
}

#[derive(Debug, Clone)]
pub struct OvkPack {
    path: PathBuf,
    index: OvkIndex,
}

impl OvkPack {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut f = File::open(&path)?;
        let file_len = f.metadata()?.len();

        let index = OvkIndex::read(file_len, |offset, length| {
            f.seek(SeekFrom::Start(offset))?;
            let mut bytes = vec![0; length];
            f.read_exact(&mut bytes)?;
            Ok(bytes)
        })?;
        Ok(Self { path, index })
    }

    pub fn entries(&self) -> &[OvkEntry] {
        self.index.entries()
    }

    pub fn get(&self, idx: usize) -> Option<OvkEntry> {
        self.index.entries().get(idx).copied()
    }

    /// Create a bounded reader for an entry.
    pub fn open_entry_stream(&self, idx: usize) -> Result<BoundedFile> {
        let e = self
            .index
            .entries
            .get(idx)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("OVK: entry index out of range: {idx}"))?;
        if e.size == 0 {
            bail!("OVK: entry[{idx}] has zero size");
        }
        BoundedFile::open(&self.path, e.offset as u64, e.size as u64, None)
    }

    /// Extract an entry into memory.
    pub fn extract_entry(&self, idx: usize) -> Result<Vec<u8>> {
        self.open_entry_stream(idx)?.read_all()
    }

    /// Decode an entry (expected to be Ogg/Vorbis) into interleaved PCM16.
    pub fn decode_entry_vorbis_pcm16(&self, idx: usize) -> Result<vorbis::Pcm16> {
        let s = self.open_entry_stream(idx)?;
        vorbis::decode_ogg_vorbis_reader(s)
    }

    /// Decode an entry (expected to be Ogg/Vorbis) and return a WAV (PCM16) buffer.
    pub fn decode_entry_vorbis_wav(&self, idx: usize) -> Result<Vec<u8>> {
        let s = self.open_entry_stream(idx)?;
        vorbis::decode_ogg_vorbis_reader_to_wav(s)
    }
}

/// OWP: XOR-obfuscated Ogg file. The original engine uses key 0x39.
#[derive(Debug, Clone)]
pub struct OwpFile {
    path: PathBuf,
    file_len: u64,
    pub xor_key: u8,
}

impl OwpFile {
    pub const DEFAULT_XOR_KEY: u8 = 0x39;

    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file_len = File::open(&path)?.metadata()?.len();
        Ok(Self {
            path,
            file_len,
            xor_key: Self::DEFAULT_XOR_KEY,
        })
    }

    pub fn open_stream(&self) -> Result<BoundedFile> {
        BoundedFile::open(&self.path, 0, self.file_len, Some(self.xor_key))
    }

    pub fn decrypt_to_vec(&self) -> Result<Vec<u8>> {
        self.open_stream()?.read_all()
    }

    /// Decode the XORed Ogg/Vorbis file into interleaved PCM16.
    pub fn decode_vorbis_pcm16(&self) -> Result<vorbis::Pcm16> {
        let s = self.open_stream()?;
        vorbis::decode_ogg_vorbis_reader(s)
    }

    /// Decode the XORed Ogg/Vorbis file and return a WAV (PCM16) buffer.
    pub fn decode_vorbis_wav(&self) -> Result<Vec<u8>> {
        let s = self.open_stream()?;
        vorbis::decode_ogg_vorbis_reader_to_wav(s)
    }
}
