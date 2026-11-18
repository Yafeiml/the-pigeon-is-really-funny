use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::Path,
};
use windows::{
    Win32::{
        Foundation::{E_INVALIDARG, HANDLE},
        System::{
            Memory::{
                CreateFileMappingW, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
                PAGE_READONLY, UnmapViewOfFile,
            },
            SystemInformation::{GetSystemInfo, SYSTEM_INFO},
        },
    },
    core::{Error, Result},
};

pub(super) const CLIPS: [&str; 6] = ["walk", "stalk", "settle", "turn", "look", "peck"];
const WINDOW_BYTES: usize = 8 * 1024 * 1024;
const MAX_FRAME_BYTES: u32 = 1024 * 1024;
const MAGIC: &[u8; 8] = b"PIGNAPK1";

fn invalid() -> Error {
    Error::new(
        E_INVALIDARG,
        "Invalid or incomplete embedded pigeon animation",
    )
}

// One cumulative offset per PNG plus an end offset. Both endpoints must be inside the payload.
fn read_index(file: &mut (impl Read + Seek)) -> Result<(u64, Vec<u32>)> {
    let count = CLIPS
        .iter()
        .map(|clip| crate::motion::sprite_frame_count(clip, true) as usize)
        .sum::<usize>();
    let index_bytes = (count + 1) * 4;
    let mut tail = vec![0; index_bytes + 16];
    let index_start = file.seek(SeekFrom::End(-(tail.len() as i64)))?;
    file.read_exact(&mut tail)?;
    if &tail[index_bytes..index_bytes + 8] != MAGIC {
        return Err(invalid());
    }
    let data_start = u64::from_le_bytes(tail[index_bytes + 8..].try_into().unwrap());
    let offsets: Vec<_> = tail[..index_bytes]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|n| u32::from_le_bytes(*n))
        .collect();
    if data_start == 0
        || offsets[0] != 0
        || data_start.checked_add(u64::from(offsets[count])) != Some(index_start)
        || offsets
            .windows(2)
            .any(|p| !matches!(p[1].checked_sub(p[0]), Some(1..=MAX_FRAME_BYTES)))
    {
        return Err(invalid());
    }
    Ok((data_start, offsets))
}

struct View {
    address: MEMORY_MAPPED_VIEW_ADDRESS,
    start: u64,
    len: usize,
}

impl Drop for View {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(self.address);
        }
    }
}

pub(super) struct PackedFrames {
    mapping: OwnedHandle,
    view: Option<View>,
    granularity: u64,
    data_start: u64,
    data_end: u64,
    offsets: Vec<u32>,
}

impl PackedFrames {
    pub(super) fn open(executable: &Path) -> Result<Self> {
        let mut file = File::open(executable)?;
        let (data_start, offsets) = read_index(&mut file)?;
        let data_end = data_start + u64::from(*offsets.last().unwrap());
        let mut info = SYSTEM_INFO::default();
        unsafe {
            GetSystemInfo(&mut info);
        }
        let granularity = u64::from(info.dwAllocationGranularity);
        if granularity == 0 || granularity + u64::from(MAX_FRAME_BYTES) > WINDOW_BYTES as u64 {
            return Err(invalid());
        }
        let mapping = unsafe {
            let handle = CreateFileMappingW(
                HANDLE(file.as_raw_handle()),
                None,
                PAGE_READONLY,
                0,
                0,
                None,
            )?;
            OwnedHandle::from_raw_handle(handle.0)
        };
        Ok(Self {
            mapping,
            view: None,
            granularity,
            data_start,
            data_end,
            offsets,
        })
    }

    pub(super) fn frame(&mut self, directory: &str, index: u32) -> Result<&[u8]> {
        let mut first = 0;
        for clip in CLIPS {
            let count = crate::motion::sprite_frame_count(clip, true);
            if clip == directory {
                if index >= count {
                    return Err(invalid());
                }
                let entry = (first + index) as usize;
                let start = self.data_start + u64::from(self.offsets[entry]);
                let end = self.data_start + u64::from(self.offsets[entry + 1]);
                return self.bytes(start, end);
            }
            first += count;
        }
        Err(invalid())
    }

    fn bytes(&mut self, start: u64, end: u64) -> Result<&[u8]> {
        if !self
            .view
            .as_ref()
            .is_some_and(|v| v.start <= start && end <= v.start + v.len as u64)
        {
            self.view = None;
            let aligned = start - start % self.granularity;
            let len = (self.data_end - aligned).min(WINDOW_BYTES as u64) as usize;
            let address = unsafe {
                MapViewOfFile(
                    HANDLE(self.mapping.as_raw_handle()),
                    FILE_MAP_READ,
                    (aligned >> 32) as u32,
                    aligned as u32,
                    len,
                )
            };
            if address.Value.is_null() {
                return Err(Error::from_thread());
            }
            self.view = Some(View {
                address,
                start: aligned,
                len,
            });
        }
        let view = self.view.as_ref().unwrap();
        // The borrowed slice pins this view until WIC finishes copying the decoded pixels.
        let mapped =
            unsafe { std::slice::from_raw_parts(view.address.Value.cast::<u8>(), view.len) };
        mapped
            .get((start - view.start) as usize..(end - view.start) as usize)
            .ok_or_else(invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn rejects_truncated_overflowing_and_overlapping_frame_tables() {
        let count: u32 = CLIPS
            .iter()
            .map(|clip| crate::motion::sprite_frame_count(clip, true))
            .sum();
        let mut file = vec![0; 1 + count as usize];
        let table = file.len();
        for offset in 0..=count {
            file.extend(offset.to_le_bytes());
        }
        file.extend(MAGIC);
        file.extend(1u64.to_le_bytes());
        let parse = |bytes: &[u8]| read_index(&mut Cursor::new(bytes));
        assert_eq!(parse(&file).unwrap().1.len(), count as usize + 1);
        assert!(parse(&file[..file.len() - 1]).is_err());
        let mut overlapping = file.clone();
        overlapping[table + 4..table + 8].copy_from_slice(&0u32.to_le_bytes());
        assert!(parse(&overlapping).is_err());
        let mut oversized = file.clone();
        oversized[table + 4..table + 8].copy_from_slice(&(MAX_FRAME_BYTES + 1).to_le_bytes());
        assert!(parse(&oversized).is_err());
        let footer_start = file.len() - 8;
        file[footer_start..].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(parse(&file).is_err());
    }
}
