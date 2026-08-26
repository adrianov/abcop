//! Hybrid file input shared by the analysis pipeline: memory-map large
//! files (zero-copy from the page cache), plain pre-sized read for small
//! ones where mmap setup would cost more than the copy.

use std::fs;

/// Files at least this large are read through a read-only mmap.
const MMAP_THRESHOLD: u64 = 64 * 1024;

pub(crate) enum SrcBuf {
    Heap(Vec<u8>),
    Mapped(memmap2::Mmap),
}

impl std::ops::Deref for SrcBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            SrcBuf::Heap(b) => b,
            SrcBuf::Mapped(m) => m,
        }
    }
}

pub(crate) fn load_src(path: &std::path::Path) -> std::io::Result<SrcBuf> {
    let file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len >= MMAP_THRESHOLD {
        // SAFETY: read-only mapping of a regular file opened for reading.
        let map = unsafe { memmap2::Mmap::map(&file)? };
        return Ok(SrcBuf::Mapped(map));
    }
    let mut buf = Vec::with_capacity(len as usize);
    {
        use std::io::Read;
        (&file).read_to_end(&mut buf)?;
    }
    Ok(SrcBuf::Heap(buf))
}
