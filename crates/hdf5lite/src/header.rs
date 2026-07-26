//! Object headers and the messages inside them: dataspace, datatype, layout, filters, attributes.

use crate::read::Cur;
use crate::{Error, Result};

/// How a dataset's bytes are stored.
#[derive(Debug, Clone)]
pub enum Layout {
    Contiguous {
        addr: u64,
        size: u64,
    },
    Chunked {
        /// Address of the version-1 B-tree that indexes the chunks.
        btree_addr: u64,
        /// Chunk shape in elements (the trailing element-size entry is stripped).
        dims: Vec<u32>,
    },
}

/// The on-disk numeric type of a dataset's elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Datatype {
    Int { size: usize, signed: bool },
    Float { size: usize },
}

impl Datatype {
    /// Width of one element in bytes.
    pub fn size_of(self) -> usize {
        match self {
            Datatype::Int { size, .. } | Datatype::Float { size } => size,
        }
    }

    /// Read one element as `f64`.
    fn read(self, b: &[u8]) -> f64 {
        match self {
            Datatype::Float { size: 4 } => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            Datatype::Float { size: 8 } => {
                f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
            }
            Datatype::Int { size: 1, signed } => {
                if signed {
                    b[0] as i8 as f64
                } else {
                    b[0] as f64
                }
            }
            Datatype::Int { size: 2, signed } => {
                let v = u16::from_le_bytes([b[0], b[1]]);
                if signed {
                    v as i16 as f64
                } else {
                    v as f64
                }
            }
            Datatype::Int { size: 4, signed } => {
                let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                if signed {
                    v as i32 as f64
                } else {
                    v as f64
                }
            }
            Datatype::Int { size: 8, signed } => {
                let v = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
                if signed {
                    v as i64 as f64
                } else {
                    v as f64
                }
            }
            _ => f64::NAN,
        }
    }
}

/// The filter pipeline, in the order the writer applied it.
#[derive(Debug, Clone, Default)]
pub struct Filters {
    /// Filter ids, outermost last (decode runs them in reverse).
    ids: Vec<u16>,
}

impl Filters {
    /// Undo the pipeline for one chunk. `expect` is the uncompressed size, `elem` the element
    /// width that the shuffle filter interleaved by.
    pub(crate) fn decode(&self, raw: &[u8], expect: usize, elem: usize) -> Result<Vec<u8>> {
        let mut buf = raw.to_vec();
        for &id in self.ids.iter().rev() {
            buf = match id {
                1 => inflate(&buf, expect)?,
                2 => unshuffle(&buf, elem),
                // Fletcher32 appends a checksum we simply don't verify; dropping the trailing
                // four bytes is what "reading past it" means.
                3 => {
                    let n = buf.len().saturating_sub(4);
                    buf.truncate(n);
                    buf
                }
                other => {
                    return Err(Error::Unsupported(format!("filter id {other}")));
                }
            };
        }
        Ok(buf)
    }
}

fn inflate(raw: &[u8], expect: usize) -> Result<Vec<u8>> {
    use flate2::bufread::ZlibDecoder;
    use std::io::Read;
    let mut out = Vec::with_capacity(expect);
    ZlibDecoder::new(raw)
        .read_to_end(&mut out)
        .map_err(|e| Error::Filter(format!("deflate: {e}")))?;
    Ok(out)
}

/// Undo the shuffle filter: bytes were grouped by significance (all byte-0s, then all byte-1s…)
/// so that similar magnitudes sit together and deflate has something to chew on.
fn unshuffle(buf: &[u8], elem: usize) -> Vec<u8> {
    if elem <= 1 || buf.len() < elem {
        return buf.to_vec();
    }
    let n = buf.len() / elem;
    let mut out = vec![0u8; buf.len()];
    for i in 0..elem {
        for j in 0..n {
            out[j * elem + i] = buf[i * n + j];
        }
    }
    // A trailing partial element is left in place, exactly as the filter wrote it.
    let done = n * elem;
    out[done..].copy_from_slice(&buf[done..]);
    out
}

/// A dataset: its shape, type, storage and the attributes needed to unpack it.
#[derive(Debug, Clone)]
pub struct Dataset {
    pub dims: Vec<u64>,
    pub dtype: Datatype,
    pub layout: Layout,
    pub filters: Filters,
    pub scale_factor: Option<f64>,
    pub add_offset: Option<f64>,
    pub fill_value: Option<f64>,
}

impl Dataset {
    pub fn element_count(&self) -> usize {
        self.dims.iter().map(|&n| n as usize).product::<usize>()
    }

    /// Turn raw bytes into unpacked values: fill values become `NaN`, then
    /// `value * scale_factor + add_offset` when the file says to.
    pub fn decode(&self, raw: &[u8]) -> Vec<f64> {
        let sz = self.dtype.size_of();
        let n = self.element_count().min(raw.len() / sz.max(1));
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let v = self.dtype.read(&raw[i * sz..(i + 1) * sz]);
            // Compare before unpacking: the fill value is stored in the on-disk representation.
            if self.fill_value.is_some_and(|f| f == v) {
                out.push(f64::NAN);
                continue;
            }
            let s = self.scale_factor.unwrap_or(1.0);
            let a = self.add_offset.unwrap_or(0.0);
            out.push(v * s + a);
        }
        out
    }
}

/// Object header message types we act on. Everything else is skipped by its recorded length.
const MSG_DATASPACE: u8 = 1;
const MSG_DATATYPE: u8 = 3;
const MSG_FILL: u8 = 5;
const MSG_LAYOUT: u8 = 8;
const MSG_FILTER: u8 = 11;
const MSG_ATTRIBUTE: u8 = 12;
const MSG_CONTINUATION: u8 = 16;
/// Points at the fractal heap holding attributes once there are too many to inline.
const MSG_ATTRIBUTE_INFO: u8 = 21;

/// One message located inside an object header.
struct Msg {
    kind: u8,
    start: usize,
}

/// Walk an object header (following continuation blocks) and collect its messages.
fn messages(d: &[u8], addr: u64) -> Result<Vec<Msg>> {
    let mut out = Vec::new();
    let mut queue = vec![(addr, None::<u64>)];
    let mut guard = 0;
    // Whether messages carry a creation-order field is an object-header-wide flag; continuation
    // blocks inherit it and can't re-declare it.
    let mut hdr_creation_order = false;
    while let Some((at, len)) = queue.pop() {
        guard += 1;
        if guard > 64 {
            return Err(Error::Unsupported(
                "object header with >64 continuations".into(),
            ));
        }
        let mut c = Cur::new(d, at as usize, "object header");
        let body_end;
        let creation_order;
        match len {
            // The first block carries the OHDR prologue.
            None => {
                c.sig(b"OHDR", "object header")?;
                let version = c.u8()?;
                if version != 2 {
                    return Err(Error::Unsupported(format!(
                        "object header version {version}"
                    )));
                }
                let flags = c.u8()?;
                if flags & 0x20 != 0 {
                    c.skip(16)?; // access/mod/change/birth times
                }
                if flags & 0x10 != 0 {
                    c.skip(4)?; // attribute phase-change limits
                }
                let size_len = 1usize << (flags & 0x03);
                let chunk = c.uint(size_len)? as usize;
                creation_order = flags & 0x04 != 0;
                hdr_creation_order = creation_order;
                body_end = c.p + chunk;
            }
            // Continuations repeat the message stream under an OCHK signature.
            Some(n) => {
                c.sig(b"OCHK", "continuation block")?;
                creation_order = hdr_creation_order;
                body_end = at as usize + n as usize;
            }
        }
        // The last four bytes of every block are a checksum, not a message.
        while c.p + 4 < body_end.min(d.len()) {
            let kind = c.u8()?;
            let size = c.u16()? as usize;
            let _flags = c.u8()?;
            if creation_order {
                c.skip(2)?;
            }
            let start = c.p;
            if start + size > d.len() {
                break;
            }
            if kind == MSG_CONTINUATION {
                let mut cc = Cur::new(d, start, "continuation");
                let caddr = cc.u64()?;
                let clen = cc.u64()?;
                queue.push((caddr, Some(clen)));
            } else {
                out.push(Msg { kind, start });
            }
            c.p = start + size;
        }
    }
    Ok(out)
}

/// Read one dataset's metadata from its object header.
pub(crate) fn parse_dataset(d: &[u8], addr: u64) -> Result<Dataset> {
    let msgs = messages(d, addr)?;
    let mut dims = None;
    let mut dtype = None;
    let mut layout = None;
    let mut filters = Filters::default();
    let mut fill_value = None;
    let mut scale_factor = None;
    let mut add_offset = None;
    let mut fill_attr = None;
    let mut attrs: Vec<(String, Option<f64>)> = Vec::new();

    for m in &msgs {
        match m.kind {
            MSG_DATASPACE => dims = Some(parse_dataspace(d, m.start)?),
            MSG_DATATYPE => dtype = Some(parse_datatype(d, m.start)?),
            MSG_LAYOUT => layout = Some(parse_layout(d, m.start)?),
            MSG_FILTER => filters = parse_filters(d, m.start)?,
            MSG_FILL => fill_value = parse_fill(d, m.start).ok().flatten(),
            MSG_ATTRIBUTE => {
                if let Ok((name, value, _)) = parse_attribute(d, m.start) {
                    attrs.push((name, value));
                }
            }
            // Datasets with more than a handful of attributes move them into a fractal heap.
            MSG_ATTRIBUTE_INFO => {
                if let Ok(heap) = attribute_info_heap(d, m.start) {
                    attrs.extend(crate::heap::dense_attributes(d, heap));
                }
            }
            _ => {}
        }
    }

    for (name, value) in attrs {
        match name.as_str() {
            "scale_factor" => scale_factor = value,
            "add_offset" => add_offset = value,
            "_FillValue" => fill_attr = value,
            _ => {}
        }
    }

    let dtype = dtype.ok_or_else(|| Error::Unsupported("dataset with no datatype".into()))?;
    // The fill VALUE message holds the raw pattern; the _FillValue ATTRIBUTE is the typed one and
    // is what netCDF actually means, so it wins when both are present.
    let fill = fill_attr.or_else(|| {
        fill_value.map(|bytes: Vec<u8>| {
            if bytes.len() >= dtype.size_of() {
                dtype.read(&bytes)
            } else {
                f64::NAN
            }
        })
    });

    Ok(Dataset {
        dims: dims.unwrap_or_default(),
        dtype,
        layout: layout.ok_or_else(|| Error::Unsupported("dataset with no layout".into()))?,
        filters,
        scale_factor,
        add_offset,
        fill_value: fill.filter(|f| !f.is_nan()),
    })
}

fn parse_dataspace(d: &[u8], at: usize) -> Result<Vec<u64>> {
    let mut c = Cur::new(d, at, "dataspace");
    let version = c.u8()?;
    let rank = c.u8()? as usize;
    let flags = c.u8()?;
    match version {
        1 => c.skip(5)?, // reserved
        2 => c.skip(1)?, // dataspace type
        v => return Err(Error::Unsupported(format!("dataspace version {v}"))),
    }
    let mut dims = Vec::with_capacity(rank);
    for _ in 0..rank {
        dims.push(c.u64()?);
    }
    // Max dims follow when flagged; the current extent is what we read.
    let _ = flags;
    Ok(dims)
}

/// The element width of any datatype message, regardless of class.
fn datatype_size(d: &[u8], at: usize) -> Result<usize> {
    let mut c = Cur::new(d, at, "datatype");
    c.skip(4)?; // class/version byte plus three flag bytes
    Ok(c.u32()? as usize)
}

fn parse_datatype(d: &[u8], at: usize) -> Result<Datatype> {
    let mut c = Cur::new(d, at, "datatype");
    let cv = c.u8()?;
    let class = cv & 0x0f;
    let flags0 = c.u8()?;
    let _flags1 = c.u8()?;
    let _flags2 = c.u8()?;
    let size = c.u32()? as usize;
    // Bit 0 of the first flags byte is the byte order: 0 little-endian, 1 big-endian.
    if flags0 & 0x01 != 0 {
        return Err(Error::Unsupported("big-endian datatype".into()));
    }
    match class {
        0 => Ok(Datatype::Int {
            size,
            // Bit 3 selects two's-complement (signed) over unsigned.
            signed: flags0 & 0x08 != 0,
        }),
        1 => Ok(Datatype::Float { size }),
        other => Err(Error::Unsupported(format!("datatype class {other}"))),
    }
}

fn parse_fill(d: &[u8], at: usize) -> Result<Option<Vec<u8>>> {
    let mut c = Cur::new(d, at, "fill value");
    let version = c.u8()?;
    match version {
        1 | 2 => {
            c.skip(3)?; // alloc time, fill time, defined flag
            let size = c.u32()? as usize;
            Ok(Some(c.bytes(size)?.to_vec()))
        }
        3 => {
            let flags = c.u8()?;
            // Bit 5 says a fill value is actually present.
            if flags & 0x20 == 0 {
                return Ok(None);
            }
            let size = c.u32()? as usize;
            Ok(Some(c.bytes(size)?.to_vec()))
        }
        v => Err(Error::Unsupported(format!("fill value version {v}"))),
    }
}

fn parse_layout(d: &[u8], at: usize) -> Result<Layout> {
    let mut c = Cur::new(d, at, "data layout");
    let version = c.u8()?;
    if version != 3 {
        return Err(Error::Unsupported(format!("data layout version {version}")));
    }
    let class = c.u8()?;
    match class {
        1 => {
            let addr = c.u64()?;
            let size = c.u64()?;
            Ok(Layout::Contiguous { addr, size })
        }
        2 => {
            // Dimensionality counts the chunk dims plus a trailing element-size entry.
            let rank = c.u8()? as usize;
            let btree_addr = c.u64()?;
            let mut dims = Vec::with_capacity(rank.saturating_sub(1));
            for _ in 0..rank.saturating_sub(1) {
                dims.push(c.u32()?);
            }
            let _elem = c.u32()?;
            Ok(Layout::Chunked { btree_addr, dims })
        }
        0 => Err(Error::Unsupported("compact layout".into())),
        other => Err(Error::Unsupported(format!("layout class {other}"))),
    }
}

fn parse_filters(d: &[u8], at: usize) -> Result<Filters> {
    let mut c = Cur::new(d, at, "filter pipeline");
    let version = c.u8()?;
    let n = c.u8()? as usize;
    if version == 1 {
        c.skip(6)?;
    } else if version != 2 {
        return Err(Error::Unsupported(format!(
            "filter pipeline version {version}"
        )));
    }
    let mut ids = Vec::with_capacity(n);
    for _ in 0..n {
        let id = c.u16()?;
        // v1 always stores the name length; v2 only does so for ids above the reserved range.
        let name_len = if version == 1 || id >= 256 {
            c.u16()? as usize
        } else {
            0
        };
        let _flags = c.u16()?;
        let nvals = c.u16()? as usize;
        if name_len > 0 {
            // Names are padded to a multiple of eight in v1.
            let pad = if version == 1 {
                name_len.div_ceil(8) * 8
            } else {
                name_len
            };
            c.skip(pad)?;
        }
        c.skip(nvals * 4)?;
        if version == 1 && nvals % 2 == 1 {
            c.skip(4)?; // padding to an even number of client values
        }
        ids.push(id);
    }
    Ok(Filters { ids })
}

/// Parse an attribute message: its name, its first value (when numeric), and the offset just past
/// it — dense attributes are stored end to end and the caller needs to know where the next begins.
pub(crate) fn parse_attribute(d: &[u8], at: usize) -> Result<(String, Option<f64>, usize)> {
    let mut c = Cur::new(d, at, "attribute");
    let version = c.u8()?;
    let (name_len, space_len, type_len, name_pad, other_pad);
    match version {
        1 => {
            c.skip(1)?;
            name_len = c.u16()? as usize;
            type_len = c.u16()? as usize;
            space_len = c.u16()? as usize;
            // v1 pads each section to eight bytes.
            name_pad = name_len.div_ceil(8) * 8;
            other_pad = true;
        }
        2 | 3 => {
            let _flags = c.u8()?;
            name_len = c.u16()? as usize;
            type_len = c.u16()? as usize;
            space_len = c.u16()? as usize;
            if version == 3 {
                c.skip(1)?; // name character-set encoding
            }
            name_pad = name_len;
            other_pad = false;
        }
        v => return Err(Error::Unsupported(format!("attribute version {v}"))),
    }
    let raw_name = c.bytes(name_pad)?;
    let name = String::from_utf8_lossy(raw_name)
        .trim_end_matches('\0')
        .to_string();

    let type_at = c.p;
    let type_span = if other_pad {
        type_len.div_ceil(8) * 8
    } else {
        type_len
    };
    c.skip(type_span)?;
    let space_span = if other_pad {
        space_len.div_ceil(8) * 8
    } else {
        space_len
    };
    c.skip(space_span)?;

    let space_at = c.p - space_span;
    let data_at = c.p;
    // Anything we can't read as a number (strings, compounds) is still a valid attribute — we
    // just have no use for its value, but we DO need its length to find the next one.
    let dt = parse_datatype(d, type_at).ok();
    let count = parse_dataspace(d, space_at)
        .map(|dims| dims.iter().map(|&n| n as usize).product::<usize>().max(1))
        .unwrap_or(1);
    // Advance by the on-disk element width whatever the class is — string attributes
    // (`standard_name`, `units`) sit between the numeric ones, and mis-measuring them desyncs the
    // whole dense-attribute walk.
    let raw_sz = datatype_size(d, type_at).unwrap_or(0);
    let sz = dt.map(|t| t.size_of()).unwrap_or(0);
    let end = data_at + count.saturating_mul(raw_sz);
    let value = match dt {
        Some(t) if sz > 0 && data_at + sz <= d.len() => Some(t.read(&d[data_at..data_at + sz])),
        _ => None,
    };
    Ok((name, value, end))
}

/// Read the Attribute Info message and return the fractal heap holding the dense attributes.
fn attribute_info_heap(d: &[u8], at: usize) -> Result<u64> {
    let mut c = Cur::new(d, at, "attribute info");
    let _version = c.u8()?;
    let flags = c.u8()?;
    if flags & 0x01 != 0 {
        c.skip(2)?; // maximum creation index
    }
    let heap = c.u64()?;
    if heap == u64::MAX {
        return Err(Error::NotFound("attribute heap".into()));
    }
    Ok(heap)
}
