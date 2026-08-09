//! Byte-level primitives: the cursor, the superblock, and assembling a dataset's raw bytes.

use crate::header::{Dataset, Layout};
use crate::{btree, Error, Result};

/// A bounds-checked little-endian cursor. HDF5 is little-endian on every platform NOAA writes on,
/// and the datatype messages in these files confirm it per-dataset.
pub(crate) struct Cur<'a> {
    pub d: &'a [u8],
    pub p: usize,
    pub what: &'static str,
}

impl<'a> Cur<'a> {
    pub fn new(d: &'a [u8], p: usize, what: &'static str) -> Cur<'a> {
        Cur { d, p, what }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.p.checked_add(n).ok_or(Error::Truncated {
            what: self.what,
            need: n,
            have: 0,
        })?;
        if end > self.d.len() {
            return Err(Error::Truncated {
                what: self.what,
                need: end,
                have: self.d.len(),
            });
        }
        let s = &self.d[self.p..end];
        self.p = end;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u64(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }

    /// An unsigned value of `n` bytes — HDF5 sizes several fields by a width stored elsewhere.
    pub fn uint(&mut self, n: usize) -> Result<u64> {
        let b = self.take(n)?;
        let mut a = [0u8; 8];
        a[..n.min(8)].copy_from_slice(&b[..n.min(8)]);
        Ok(u64::from_le_bytes(a))
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        self.take(n)
    }

    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }

    pub fn sig(&mut self, want: &[u8; 4], what: &'static str) -> Result<()> {
        let b = self.take(4)?;
        if b != want {
            let mut f = [0u8; 4];
            f.copy_from_slice(b);
            return Err(Error::BadSignature { what, found: f });
        }
        Ok(())
    }
}

const MAGIC: [u8; 8] = [0x89, b'H', b'D', b'F', b'\r', b'\n', 0x1a, b'\n'];

/// Validate the superblock and return the root group's object-header address.
pub(crate) fn superblock_root(d: &[u8]) -> Result<u64> {
    if d.len() < 8 || d[..8] != MAGIC {
        return Err(Error::BadSignature {
            what: "file",
            found: [
                *d.first().unwrap_or(&0),
                *d.get(1).unwrap_or(&0),
                *d.get(2).unwrap_or(&0),
                *d.get(3).unwrap_or(&0),
            ],
        });
    }
    let mut c = Cur::new(d, 8, "superblock");
    let version = c.u8()?;
    if version < 2 {
        // v0/v1 keep the root group in a symbol-table entry at a fixed spot near the front of the
        // file. netCDF-4 stopped writing these, but the European radar tools (ODIM_H5 via plain
        // libhdf5 defaults) still do, so the address is worth digging out.
        c.skip(4)?; // free-space version, root symbol-table version, reserved, shared-message version
        let offset_size = c.u8()? as usize;
        let _length_size = c.u8()?;
        c.skip(1)?; // reserved
        if offset_size != 8 {
            return Err(Error::Unsupported(format!(
                "{offset_size}-byte file offsets"
            )));
        }
        c.skip(2 + 2 + 4)?; // group leaf/internal node K, file consistency flags
        if version == 1 {
            c.skip(4)?; // indexed storage internal node K, plus its padding
        }
        c.skip(8 * 4)?; // base address, free-space info, end of file, driver info
        c.skip(8)?; // the root entry's link name offset — the root has no name
        return c.u64();
    }
    let offset_size = c.u8()? as usize;
    let _length_size = c.u8()?;
    if offset_size != 8 {
        return Err(Error::Unsupported(format!(
            "{offset_size}-byte file offsets"
        )));
    }
    let _flags = c.u8()?;
    let _base = c.u64()?;
    let _ext = c.u64()?;
    let _eof = c.u64()?;
    let root = c.u64()?;
    Ok(root)
}

/// Copy one decoded chunk into the dataset buffer, row by row.
///
/// GLM is one-dimensional, where this is a single memcpy; ODIM_H5 sweeps are `nrays × nbins`, so
/// the chunk's rows land at a stride. The innermost chunk dimension is contiguous in both buffers,
/// and everything outside it is walked as an odometer over the chunk's origin.
fn place_chunk(
    out: &mut [u8],
    src: &[u8],
    elem: usize,
    dset_dims: &[u64],
    chunk_dims: &[u32],
    origin: &[u64],
) {
    let rank = dset_dims.len();
    if rank == 0 || chunk_dims.len() != rank || origin.len() < rank {
        return;
    }
    // Row-major strides in elements, innermost last.
    let mut stride = vec![1usize; rank];
    for k in (0..rank.saturating_sub(1)).rev() {
        stride[k] = stride[k + 1] * dset_dims[k + 1] as usize;
    }
    let run = chunk_dims[rank - 1] as usize;
    let rows: usize = chunk_dims[..rank - 1].iter().map(|&n| n as usize).product();
    for r in 0..rows {
        let mut rem = r;
        let mut dst = origin[rank - 1] as usize;
        let mut inside = true;
        for k in (0..rank - 1).rev() {
            let c = rem % chunk_dims[k] as usize;
            rem /= chunk_dims[k] as usize;
            let i = origin[k] as usize + c;
            // Chunks are allowed to overhang the dataset's extent; those rows are padding.
            if i >= dset_dims[k] as usize {
                inside = false;
                break;
            }
            dst += i * stride[k];
        }
        if !inside {
            continue;
        }
        let (dst, src_at) = (dst * elem, r * run * elem);
        if dst >= out.len() || src_at >= src.len() {
            continue;
        }
        let n = (run * elem).min(out.len() - dst).min(src.len() - src_at);
        out[dst..dst + n].copy_from_slice(&src[src_at..src_at + n]);
    }
}

/// Gather a dataset's on-disk bytes, decompressing and reassembling chunks as needed.
pub(crate) fn raw_bytes(d: &[u8], ds: &Dataset) -> Result<Vec<u8>> {
    let elem = ds.dtype.size_of();
    let total = ds.element_count() * elem;
    match &ds.layout {
        Layout::Contiguous { addr, size } => {
            let end = (*addr as usize)
                .checked_add(*size as usize)
                .ok_or(Error::Truncated {
                    what: "contiguous data",
                    need: 0,
                    have: d.len(),
                })?;
            if end > d.len() {
                return Err(Error::Truncated {
                    what: "contiguous data",
                    need: end,
                    have: d.len(),
                });
            }
            Ok(d[*addr as usize..end].to_vec())
        }
        Layout::Chunked { btree_addr, dims } => {
            // Chunk dims are stored with the element size appended; the last entry isn't a
            // dimension.
            let chunk_elems: usize = dims.iter().map(|&n| n as usize).product::<usize>().max(1);
            let mut out = vec![0u8; total];
            if *btree_addr == u64::MAX {
                // No chunks were ever allocated — an empty dataset, not an error.
                return Ok(out);
            }
            for chunk in btree::chunks(d, *btree_addr, dims.len())? {
                let end = (chunk.addr as usize).saturating_add(chunk.size as usize);
                if end > d.len() {
                    return Err(Error::Truncated {
                        what: "chunk data",
                        need: end,
                        have: d.len(),
                    });
                }
                let raw = &d[chunk.addr as usize..end];
                let decoded = ds.filters.decode(raw, chunk_elems * elem, elem)?;
                place_chunk(&mut out, &decoded, elem, &ds.dims, dims, &chunk.offset);
            }
            Ok(out)
        }
    }
}
