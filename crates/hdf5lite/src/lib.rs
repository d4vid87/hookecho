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
//! Since then it has grown a second consumer: ODIM_H5, the European radar volume format, which
//! libhdf5 writes with its *oldest* on-disk structures. So both eras are read.
//!
//! - Superblock versions 0–3 (4- or 8-byte offsets/lengths).
//! - Object header versions 1 and 2 (`OHDR`), both with continuation blocks.
//! - Groups whose links live in a fractal heap (`FRHP`/`FHIB`/`FHDB`), and old-style symbol-table
//!   groups (`TREE`/`SNOD` plus a `HEAP` of names). Nested paths resolve through either.
//! - Dataspace v1–v2, datatype v1 (little- or big-endian fixed-point and floating-point), fill value, filter pipeline.
//! - Data layout v3: contiguous, and chunked indexed by a version-1 B-tree, of any rank.
//! - The `shuffle` and `deflate` filters.
//! - Attributes on datasets *and* groups — numeric, fixed-length string, and variable-length
//!   string (via the `GCOL` global heap). ODIM keeps its entire metadata model in group
//!   attributes, so groups matter as much as datasets here.
//!
//! Not supported, and erroring rather than guessed at: compact layout, huge or tiny fractal-heap
//! objects, filtered heaps, compound/enum datatypes, and soft or external
//! links.

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

/// An attribute value. Numeric arrays are read in full only when explicitly requested.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Numbers(Vec<f64>),
    Str(String),
}

impl Value {
    /// The value as a number, or `None` if it's a string.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(v) => Some(*v),
            Value::Numbers(v) => v.first().copied(),
            Value::Str(_) => None,
        }
    }

    /// The value as a string, or `None` if it's a number.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// An open HDF5 file. The whole file is held in memory — GLM granules are a few hundred KB, and
/// the alternative (seeking I/O threaded through every parser) buys nothing at that size.
pub struct File {
    data: Vec<u8>,
    /// Address of the root group's object header.
    root: u64,
    /// Top-level object name to its object-header address.
    objects: HashMap<String, u64>,
}

impl File {
    /// Parse `data` as an HDF5 file and index its root group.
    pub fn open(data: Vec<u8>) -> Result<File> {
        let root = read::superblock_root(&data)?;
        let objects = heap::links(&data, root)?;
        if objects.is_empty() {
            return Err(Error::Unsupported("file with an empty root group".into()));
        }
        Ok(File {
            data,
            root,
            objects,
        })
    }

    /// Every top-level object name, sorted — useful for diagnostics and for the golden tests.
    pub fn names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.objects.keys().map(String::as_str).collect();
        v.sort_unstable();
        v
    }

    /// The object-header address of a slash-separated path, `""` meaning the root group.
    fn resolve(&self, path: &str) -> Result<u64> {
        let mut addr = self.root;
        for part in path.split('/').filter(|s| !s.is_empty()) {
            addr = *heap::links(&self.data, addr)?
                .get(part)
                .ok_or_else(|| Error::NotFound(path.to_string()))?;
        }
        Ok(addr)
    }

    /// The names of a group's children, sorted. ODIM numbers its sweeps `dataset1`, `dataset2`, …
    /// without recording how many there are, so listing is how you find out.
    pub fn children(&self, path: &str) -> Result<Vec<String>> {
        let mut v: Vec<String> = heap::links(&self.data, self.resolve(path)?)?
            .into_keys()
            .collect();
        v.sort();
        Ok(v)
    }

    /// Every attribute on the object at `path`, group or dataset.
    pub fn attributes(&self, path: &str) -> Result<HashMap<String, Value>> {
        self.attributes_with_arrays(path, &[])
    }

    /// Read the specified one-dimensional numeric attributes only at their expected lengths.
    /// Other attributes retain their usual scalar representation.
    pub fn attributes_with_arrays(&self, path: &str, arrays: &[(&str, usize)]) -> Result<HashMap<String, Value>> {
        Ok(header::object_attributes(&self.data, self.resolve(path)?, arrays)?
            .into_iter()
            .filter_map(|(k, v)| v.map(|v| (k, v)))
            .collect())
    }

    /// Read a dataset's metadata (shape, type, filters, and any scale/offset attributes).
    pub fn dataset(&self, name: &str) -> Result<Dataset> {
        header::parse_dataset(&self.data, self.resolve(name)?)
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

    /// Sample a two-dimensional dataset before allocating its decoded grid.
    pub fn read_f64_strided_2d(&self, name: &str, stride: usize) -> Result<Vec<f64>> {
        let ds = self.dataset(name)?;
        read::strided_f64_2d(&self.data, &ds, stride.max(1))
    }
}
