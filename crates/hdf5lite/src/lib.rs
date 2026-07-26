//! A minimal, read-only HDF5 reader — just enough to open the netCDF-4 files NOAA publishes.
//!
//! # Why this exists
//!
//! GOES GLM lightning is distributed as netCDF-4, which is HDF5 underneath. The reference reader
//! is a C library, and Android builds can't take it: the app ships a pure-Rust `NativeActivity`
//! with no C toolchain in the loop. Rather than drop lightning on the phone, this reads the
//! handful of HDF5 structures the netCDF-4 writer actually emits.
//!
//! # What it deliberately does NOT do
//!
//! This is not an HDF5 implementation. It reads what was found in real GLM granules and returns a
//! clear `Unsupported` error for everything else, so an unexpected file fails loudly instead of
//! returning quiet nonsense. Supported:
//!
//! - Superblock versions 2 and 3 (8-byte offsets/lengths).
//! - Object header version 2 (`OHDR`) with continuation blocks (`OCHK`).
//! - Groups whose links live in a fractal heap (`FRHP`/`FHIB`/`FHDB`).
//! - Dataspace v1–v2, datatype v1 (fixed-point and floating-point), fill value, filter pipeline.
//! - Data layout v3: contiguous, and chunked indexed by a version-1 B-tree.
//! - The `shuffle` and `deflate` filters.
//! - Attributes with scalar or simple numeric values (`scale_factor`, `add_offset`, `_FillValue`).
//!
//! Not supported, and erroring rather than guessed at: symbol-table groups, compact layout, huge
//! or tiny fractal-heap objects, filtered heaps, compound/string/enum datatypes, and anything
//! written by a tool other than netCDF-4.

mod btree;
mod header;
mod heap;
mod read;

use std::collections::HashMap;

pub use header::{Dataset, Datatype};

/// Everything that can go wrong reading a file. Truncation and unsupported features are separate
/// on purpose: one means "this file is damaged", the other means "this reader is too small".
#[derive(Debug)]
pub enum Error {
    /// The bytes ran out where a structure was expected.
    Truncated {
        what: &'static str,
        need: usize,
        have: usize,
    },
    /// A signature didn't match — the file isn't laid out the way we expect.
    BadSignature { what: &'static str, found: [u8; 4] },
    /// A real HDF5 feature this reader doesn't implement.
    Unsupported(String),
    /// No dataset or attribute by that name.
    NotFound(String),
    /// A filter failed (corrupt compressed chunk).
    Filter(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Truncated { what, need, have } => {
                write!(
                    f,
                    "hdf5lite: truncated {what}: need {need} bytes, have {have}"
                )
            }
            Error::BadSignature { what, found } => {
                write!(f, "hdf5lite: bad {what} signature {found:?}")
            }
            Error::Unsupported(s) => write!(f, "hdf5lite: unsupported {s}"),
            Error::NotFound(s) => write!(f, "hdf5lite: no such object {s:?}"),
            Error::Filter(s) => write!(f, "hdf5lite: filter failed: {s}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// An open HDF5 file. The whole file is held in memory — GLM granules are a few hundred KB, and
/// the alternative (seeking I/O threaded through every parser) buys nothing at that size.
pub struct File {
    data: Vec<u8>,
    /// Top-level object name to its object-header address.
    objects: HashMap<String, u64>,
}

impl File {
    /// Parse `data` as an HDF5 file and index its root group.
    pub fn open(data: Vec<u8>) -> Result<File> {
        let root = read::superblock_root(&data)?;
        let mut f = File {
            data,
            objects: HashMap::new(),
        };
        f.objects = heap::root_links(&f.data, root)?;
        Ok(f)
    }

    /// Every top-level object name, sorted — useful for diagnostics and for the golden tests.
    pub fn names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.objects.keys().map(String::as_str).collect();
        v.sort_unstable();
        v
    }

    /// Read a dataset's metadata (shape, type, filters, and any scale/offset attributes).
    pub fn dataset(&self, name: &str) -> Result<Dataset> {
        let addr = *self
            .objects
            .get(name)
            .ok_or_else(|| Error::NotFound(name.to_string()))?;
        header::parse_dataset(&self.data, addr)
    }

    /// Read a dataset as `f64`, applying `scale_factor`/`add_offset` when present and mapping the
    /// fill value to `NaN`.
    ///
    /// Everything netCDF-4 stores is a number that becomes a float once unpacked, so one accessor
    /// covers every dataset this reader needs and callers don't branch on the on-disk type.
    pub fn read_f64(&self, name: &str) -> Result<Vec<f64>> {
        let ds = self.dataset(name)?;
        let raw = read::raw_bytes(&self.data, &ds)?;
        Ok(ds.decode(&raw))
    }
}
