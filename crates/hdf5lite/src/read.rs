//! Byte-level primitives: the cursor, the superblock, and assembling a dataset's raw bytes.

use crate::header::{Dataset, Layout};
use crate::{btree, Error, Result};

// Full-resolution ABI images are sampled chunk by chunk; no individual allocation needs these
// ceilings. Keep them shared by GLM, ABI and ODIM, including files with forged dimensions.
pub(crate) const MAX_CHUNK_BYTES: usize = 64 * 1024 * 1024;
const MAX_READ_BYTES: usize = 512 * 1024 * 1024;

fn byte_len(dims: impl IntoIterator<Item = u64>, elem: usize, limit: usize) -> Result<usize> {
    let size = dims.into_iter().try_fold(elem, |total, n| {
        total.checked_mul(usize::try_from(n).ok()?)
    });
    size.filter(|&size| size <= limit)
        .ok_or_else(|| Error::Unsupported("dataset or chunk dimensions exceed byte limit".into()))
}

fn chunk_bytes(ds: &Dataset, dims: &[u32]) -> Result<usize> {
    if dims.is_empty() || dims.len() != ds.dims.len() || dims.contains(&0) {
        return Err(Error::Unsupported("invalid chunk dimensions".into()));
    }
    byte_len(dims.iter().map(|&n| u64::from(n)), ds.dtype.size_of(), MAX_CHUNK_BYTES)
}

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
        if n == 0 || n > 8 {
            return Err(Error::Unsupported(format!("{n}-byte integer width")));
        }
        let b = self.take(n)?;
        let mut a = [0u8; 8];
        a[..n.min(8)].copy_from_slice(&b[..n.min(8)]);
        Ok(u64::from_le_bytes(a))
    }

    /// File addresses and HDF5 lengths use widths declared by the superblock. Small ODIM files
    /// commonly use four-byte fields; the all-ones address is the undefined-address sentinel.
    pub fn addr(&mut self) -> Result<u64> {
        let width = file_widths(self.d)?.0;
        let value = self.uint(width)?;
        Ok(if value == u64::MAX >> (8 * (8 - width)) { u64::MAX } else { value })
    }

    pub fn length(&mut self) -> Result<u64> {
        self.uint(file_widths(self.d)?.1)
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

fn file_widths(d: &[u8]) -> Result<(usize, usize)> {
    let version = *d.get(8).ok_or(Error::Truncated { what: "superblock", need: 9, have: d.len() })?;
    let at = if version < 2 { 13 } else { 9 };
    let address = *d.get(at).ok_or(Error::Truncated { what: "superblock", need: at + 2, have: d.len() })? as usize;
    let length = *d.get(at + 1).ok_or(Error::Truncated { what: "superblock", need: at + 2, have: d.len() })? as usize;
    if !matches!(address, 4 | 8) || !matches!(length, 4 | 8) {
        return Err(Error::Unsupported(format!("{address}-byte offsets and {length}-byte lengths")));
    }
    Ok((address, length))
}

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
        let length_size = c.u8()? as usize;
        c.skip(1)?; // reserved
        file_widths(d)?;
        c.skip(2 + 2 + 4)?; // group leaf/internal node K, file consistency flags
        if version == 1 {
            c.skip(4)?; // indexed storage internal node K, plus its padding
        }
        c.skip(offset_size * 4)?; // base address, free-space info, end of file, driver info
        c.skip(length_size)?; // the root entry's link name offset — the root has no name
        return c.addr();
    }
    let _offset_size = c.u8()?;
    let _length_size = c.u8()?;
    file_widths(d)?;
    let _flags = c.u8()?;
    let _base = c.addr()?;
    let _ext = c.addr()?;
    let _eof = c.addr()?;
    let root = c.addr()?;
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
    if rank == 0 || chunk_dims.len() != rank || origin.len() < rank
        || origin.iter().zip(dset_dims).any(|(&start, &size)| start >= size) {
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
        let n = run.min((dset_dims[rank - 1] - origin[rank - 1]) as usize) * elem;
        out[dst..dst + n].copy_from_slice(&src[src_at..src_at + n]);
    }
}

/// Gather a dataset's on-disk bytes, decompressing and reassembling chunks as needed.
pub(crate) fn raw_bytes(d: &[u8], ds: &Dataset) -> Result<Vec<u8>> {
    let elem = ds.dtype.size_of();
    if !matches!(elem, 1 | 2 | 4 | 8) {
        return Err(Error::Unsupported(format!("{elem}-byte datatype")));
    }
    byte_len(ds.dims.iter().copied(), 8, MAX_READ_BYTES)?;
    let total = byte_len(ds.dims.iter().copied(), elem, MAX_READ_BYTES)?;
    match &ds.layout {
        Layout::Contiguous { addr, size } => {
            if *size != total as u64 {
                return Err(Error::Filter(format!("contiguous data has {size} bytes, expected {total}")));
            }
            let start = usize::try_from(*addr).map_err(|_| Error::Unsupported("data address exceeds address space".into()))?;
            let end = start
                .checked_add(total)
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
            Ok(d[start..end].to_vec())
        }
        Layout::Chunked { btree_addr, dims } => {
            let expect = chunk_bytes(ds, dims)?;
            let mut out = vec![0u8; total];
            if *btree_addr == u64::MAX {
                // No chunks were ever allocated — an empty dataset, not an error.
                return Ok(out);
            }
            for chunk in btree::chunks(d, *btree_addr, dims.len())? {
                let start = usize::try_from(chunk.addr).map_err(|_| Error::Unsupported("chunk address exceeds address space".into()))?;
                let end = start.saturating_add(chunk.size as usize);
                if end > d.len() {
                    return Err(Error::Truncated {
                        what: "chunk data",
                        need: end,
                        have: d.len(),
                    });
                }
                let raw = &d[start..end];
                let decoded = ds.filters.decode(raw, expect, elem)?;
                place_chunk(&mut out, &decoded, elem, &ds.dims, dims, &chunk.offset);
            }
            Ok(out)
        }
    }
}

/// Read only display-grid points from a large chunked HDF5 image. Each compressed chunk is
/// expanded at most once; the full uncompressed image is never allocated.
pub(crate) fn strided_f64_2d(d: &[u8], ds: &Dataset, step: usize) -> Result<Vec<f64>> {
    let [height, width] = ds.dims.as_slice() else {
        return Err(Error::Unsupported("strided read requires a 2D dataset".into()));
    };
    let elem = ds.dtype.size_of();
    if !matches!(elem, 1 | 2 | 4 | 8) {
        return Err(Error::Unsupported(format!("{elem}-byte datatype")));
    }
    byte_len(ds.dims.iter().copied(), elem, usize::MAX)?;
    let (height, width) = (usize::try_from(*height), usize::try_from(*width));
    let (height, width) = (height.map_err(|_| Error::Unsupported("image height exceeds address space".into()))?,
        width.map_err(|_| Error::Unsupported("image width exceeds address space".into()))?);
    let step = step.max(1);
    let out_width = width.div_ceil(step);
    let out_bytes = byte_len([out_width as u64, height.div_ceil(step) as u64], 8, MAX_READ_BYTES)?;
    let mut out = vec![f64::NAN; out_bytes / 8];
    match &ds.layout {
        Layout::Chunked { btree_addr, dims } if dims.len() == 2 => {
            let expect = chunk_bytes(ds, dims)?;
            if *btree_addr == u64::MAX { return Ok(out); }
            for chunk in btree::chunks(d, *btree_addr, 2)? {
                if chunk.offset[0] >= height as u64 || chunk.offset[1] >= width as u64 { continue; }
                let (y0, x0) = (chunk.offset[0] as usize, chunk.offset[1] as usize);
                let (y1, x1) = (y0.saturating_add(dims[0] as usize).min(height), x0.saturating_add(dims[1] as usize).min(width));
                let (Some(first_y), Some(first_x)) = (y0.div_ceil(step).checked_mul(step), x0.div_ceil(step).checked_mul(step)) else { continue; };
                if first_y >= y1 || first_x >= x1 { continue; }
                let start = usize::try_from(chunk.addr).map_err(|_| Error::Unsupported("chunk address exceeds address space".into()))?;
                let end = start.checked_add(chunk.size as usize).ok_or(Error::Truncated {
                    what: "chunk data", need: usize::MAX, have: d.len(),
                })?;
                let raw = d.get(start..end).ok_or(Error::Truncated {
                    what: "chunk data", need: end, have: d.len(),
                })?;
                let decoded = ds.filters.decode(raw, expect, elem)?;
                for y in (first_y..y1).step_by(step) {
                    for x in (first_x..x1).step_by(step) {
                        let at = ((y - y0) * dims[1] as usize + (x - x0)) * elem;
                        if let Some(bytes) = decoded.get(at..at + elem) {
                            out[(y / step) * out_width + x / step] = ds.decode_one(bytes);
                        }
                    }
                }
            }
        }
        _ => {
            let raw = raw_bytes(d, ds)?;
            for y in (0..height).step_by(step) {
                for x in (0..width).step_by(step) {
                    let at = (y * width + x) * elem;
                    if let Some(bytes) = raw.get(at..at + elem) {
                        out[(y / step) * out_width + x / step] = ds.decode_one(bytes);
                    }
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::{Datatype, Filters};

    fn dataset(dims: Vec<u64>, chunk: Vec<u32>) -> Dataset {
        Dataset { dims, dtype: Datatype::Int { size: 2, signed: false, big_endian: false },
            layout: Layout::Chunked { btree_addr: u64::MAX, dims: chunk }, filters: Filters::default(),
            scale_factor: None, add_offset: None, fill_value: None }
    }

    #[test]
    fn malformed_dimensions_fail_before_allocation_in_both_read_paths() {
        for dims in [vec![0, 4], vec![4], vec![u32::MAX, u32::MAX], vec![8192, 8192]] {
            let ds = dataset(vec![4, 4], dims);
            assert!(raw_bytes(&[], &ds).is_err());
            assert!(strided_f64_2d(&[], &ds, 2).is_err());
        }
        for dims in [vec![u64::MAX, 2], vec![8193, 8193]] {
            let ds = dataset(dims, vec![4, 4]);
            assert!(raw_bytes(&[], &ds).is_err());
            assert!(strided_f64_2d(&[], &ds, 1).is_err());
        }
        let ds = dataset(vec![21696, 21696], vec![256, 256]);
        assert_eq!(strided_f64_2d(&[], &ds, 16).unwrap().len(), 1356 * 1356);
    }

    #[test]
    fn contiguous_size_must_match_shape_and_chunk_padding_stays_in_its_row() {
        let mut ds = dataset(vec![2, 2], vec![2, 2]);
        ds.layout = Layout::Contiguous { addr: 0, size: 6 };
        assert!(raw_bytes(&[0; 8], &ds).is_err());
        ds.layout = Layout::Contiguous { addr: 0, size: 8 };
        assert_eq!(raw_bytes(&[42; 8], &ds).unwrap(), [42; 8]);
        let mut output = [0; 6];
        place_chunk(&mut output, &[1, 2, 3, 4], 1, &[2, 3], &[2, 2], &[0, 2]);
        assert_eq!(output, [0, 0, 1, 0, 0, 3]);
    }
}
