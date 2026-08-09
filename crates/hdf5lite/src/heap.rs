//! Group links, in both of the shapes HDF5 stores them.
//!
//! A group written by a modern library keeps its links in a fractal heap; one written the old way
//! (which is still what libhdf5 does by default, and so what the European ODIM_H5 radar tools
//! emit) keeps them in a version-1 B-tree of symbol-table nodes with the names in a local heap.
//! Both end at the same place — a name and an object-header address — so [`links`] hides the
//! difference and everything above it works on either.
//!
//! The fractal-heap side has a shortcut worth explaining:
//!
//! A fractal heap normally needs its companion version-2 B-tree to enumerate: the B-tree records
//! hold heap IDs that encode each object's offset and length. That's a lot of machinery to find
//! forty dataset names.
//!
//! ponytail: we walk the heap's direct blocks and parse link messages back to back instead. Link
//! messages are self-describing — each one carries its own name length — so a block written in one
//! pass and never edited reads straight through. Files written once by netCDF-4 are exactly that.
//! A heap with holes from deletions would desync; the walk stops as soon as a message doesn't
//! parse, and the golden tests assert every expected name is found. Add the B-tree if a writer
//! ever hands us a fragmented heap.

use crate::read::Cur;
use crate::{Error, Result};
use std::collections::HashMap;

/// Read a group's links: name to object-header address. An empty map means an empty group, which
/// ODIM files are full of — `where` and `how` hold attributes and no children at all.
pub(crate) fn links(d: &[u8], group_hdr: u64) -> Result<HashMap<String, u64>> {
    let mut out = HashMap::new();
    for m in crate::header::messages(d, group_hdr)? {
        match m.kind {
            crate::header::MSG_LINK_INFO => {
                let mut c = Cur::new(d, m.start, "link info");
                let _v = c.u8()?;
                let flags = c.u8()?;
                if flags & 0x01 != 0 {
                    c.skip(8)?; // maximum creation index
                }
                let heap = c.u64()?;
                if heap == u64::MAX {
                    continue;
                }
                for (start, end) in direct_blocks(d, heap)? {
                    walk_links(d, start, end, &mut out);
                }
            }
            crate::header::MSG_SYMBOL_TABLE => {
                let mut c = Cur::new(d, m.start, "symbol table message");
                let btree = c.u64()?;
                let local_heap = c.u64()?;
                symbol_table_links(d, btree, local_heap, &mut out)?;
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Walk the version-1 B-tree of an old-style group. Its leaves are symbol-table nodes; the keys
/// are name offsets we never need, because every node repeats the offset per entry.
fn symbol_table_links(
    d: &[u8],
    btree: u64,
    local_heap: u64,
    out: &mut HashMap<String, u64>,
) -> Result<()> {
    if btree == u64::MAX {
        return Ok(());
    }
    let names = local_heap_data(d, local_heap)?;
    let mut stack = vec![btree];
    let mut guard = 0;
    while let Some(addr) = stack.pop() {
        if addr == u64::MAX {
            continue;
        }
        guard += 1;
        if guard > 4096 {
            return Err(Error::Unsupported("group B-tree with >4096 nodes".into()));
        }
        let mut c = Cur::new(d, addr as usize, "group B-tree node");
        c.sig(b"TREE", "B-tree")?;
        let node_type = c.u8()?;
        if node_type != 0 {
            return Err(Error::Unsupported(format!(
                "group B-tree node type {node_type}"
            )));
        }
        let level = c.u8()?;
        let entries = c.u16()? as usize;
        c.skip(8 * 2)?; // left and right sibling addresses
        for _ in 0..entries {
            let _key = c.u64()?; // offset into the local heap of the entry's first name
            let child = c.u64()?;
            if level == 0 {
                symbol_table_node(d, child, names, out)?;
            } else {
                stack.push(child);
            }
        }
    }
    Ok(())
}

/// One symbol-table node: a run of forty-byte entries, each a name offset and a header address.
fn symbol_table_node(
    d: &[u8],
    addr: u64,
    names: usize,
    out: &mut HashMap<String, u64>,
) -> Result<()> {
    let mut c = Cur::new(d, addr as usize, "symbol table node");
    c.sig(b"SNOD", "symbol table node")?;
    c.skip(2)?; // version, reserved
    let count = c.u16()? as usize;
    for _ in 0..count {
        let name_at = names + c.u64()? as usize;
        let header = c.u64()?;
        c.skip(4 + 4 + 16)?; // cache type, reserved, scratch pad
        out.insert(heap_string(d, name_at), header);
    }
    Ok(())
}

/// The file offset where a local heap's name bytes begin.
fn local_heap_data(d: &[u8], addr: u64) -> Result<usize> {
    let mut c = Cur::new(d, addr as usize, "local heap");
    c.sig(b"HEAP", "local heap")?;
    let version = c.u8()?;
    if version != 0 {
        return Err(Error::Unsupported(format!("local heap version {version}")));
    }
    c.skip(3)?; // reserved
    c.skip(8 * 2)?; // data segment size, offset of the free list head
    Ok(c.u64()? as usize)
}

/// A NUL-terminated name out of a local heap.
fn heap_string(d: &[u8], at: usize) -> String {
    let tail = d.get(at..).unwrap_or(&[]);
    let n = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
    String::from_utf8_lossy(&tail[..n]).to_string()
}

/// A direct block's object area: `(start, end)` byte offsets in the file.
type Block = (usize, usize);

/// Collect every direct block of a fractal heap, following the root indirect block when present.
fn direct_blocks(d: &[u8], heap_addr: u64) -> Result<Vec<Block>> {
    let mut c = Cur::new(d, heap_addr as usize, "fractal heap header");
    c.sig(b"FRHP", "fractal heap")?;
    let version = c.u8()?;
    if version != 0 {
        return Err(Error::Unsupported(format!(
            "fractal heap version {version}"
        )));
    }
    let _id_len = c.u16()?;
    let io_filter_len = c.u16()?;
    let flags = c.u8()?;
    let _max_managed = c.u32()?;
    c.skip(8 * 2)?; // next huge object id, huge object B-tree
    c.skip(8 * 2)?; // free space, free space manager
    c.skip(8 * 2)?; // managed space, allocated managed space
    c.skip(8)?; // direct block allocation iterator offset
    c.skip(8)?; // number of managed objects
    c.skip(8 * 2)?; // huge object size and count
    c.skip(8 * 2)?; // tiny object size and count
    let table_width = c.u16()? as usize;
    let start_block_size = c.u64()? as usize;
    let max_direct_size = c.u64()? as usize;
    let max_heap_bits = c.u16()? as usize;
    let _start_rows = c.u16()?;
    let root_addr = c.u64()?;
    let cur_rows = c.u16()? as usize;

    if io_filter_len != 0 {
        return Err(Error::Unsupported("filtered fractal heap".into()));
    }
    let offset_bytes = max_heap_bits.div_ceil(8);
    let checksummed = flags & 0x02 != 0;
    if root_addr == u64::MAX {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    if cur_rows == 0 {
        // The root block IS a direct block.
        out.push(direct_block(
            d,
            root_addr,
            start_block_size,
            offset_bytes,
            checksummed,
        )?);
        return Ok(out);
    }

    // Root indirect block: a table of child addresses, `table_width` per row. Row 0 and row 1 are
    // both `start_block_size`; each row after that doubles.
    let mut c = Cur::new(d, root_addr as usize, "indirect block");
    c.sig(b"FHIB", "indirect block")?;
    let _v = c.u8()?;
    c.skip(8)?; // heap header address
    c.skip(offset_bytes)?; // block offset
    let max_direct_rows = (max_direct_size / start_block_size).ilog2() as usize + 2;
    for row in 0..cur_rows.min(max_direct_rows) {
        let size = if row < 2 {
            start_block_size
        } else {
            start_block_size << (row - 1)
        };
        for _ in 0..table_width {
            let addr = c.u64()?;
            if addr == u64::MAX {
                continue; // an unallocated slot
            }
            match direct_block(d, addr, size, offset_bytes, checksummed) {
                Ok(b) => out.push(b),
                // One bad block shouldn't cost the rest of the group.
                Err(e) => log::debug!("hdf5lite: skipping heap block at {addr:#x}: {e}"),
            }
        }
    }
    Ok(out)
}

/// The object area of one direct block, past its own header.
fn direct_block(
    d: &[u8],
    addr: u64,
    size: usize,
    offset_bytes: usize,
    checksummed: bool,
) -> Result<Block> {
    let mut c = Cur::new(d, addr as usize, "direct block");
    c.sig(b"FHDB", "direct block")?;
    let _v = c.u8()?;
    c.skip(8)?; // heap header address
    c.skip(offset_bytes)?; // block offset
    if checksummed {
        c.skip(4)?;
    }
    let end = (addr as usize + size).min(d.len());
    if c.p >= end {
        return Err(Error::Truncated {
            what: "direct block",
            need: c.p,
            have: end,
        });
    }
    Ok((c.p, end))
}

/// Parse link messages laid end to end, stopping at the first one that doesn't look like a link.
fn walk_links(d: &[u8], start: usize, end: usize, out: &mut HashMap<String, u64>) {
    let mut p = start;
    while p < end {
        match parse_link(d, p, end) {
            Ok((name, addr, next)) => {
                out.insert(name, addr);
                p = next;
            }
            // Free space in the block reads as zeros or garbage; that's the end of the run.
            Err(_) => break,
        }
    }
}

/// One link message: returns its name, target address, and where the next one starts.
fn parse_link(d: &[u8], at: usize, end: usize) -> Result<(String, u64, usize)> {
    let mut c = Cur::new(&d[..end.min(d.len())], at, "link message");
    let version = c.u8()?;
    if version != 1 {
        return Err(Error::Unsupported(format!(
            "link message version {version}"
        )));
    }
    let flags = c.u8()?;
    let link_type = if flags & 0x08 != 0 { c.u8()? } else { 0 };
    if flags & 0x04 != 0 {
        c.skip(8)?; // creation order
    }
    if flags & 0x10 != 0 {
        c.skip(1)?; // link name character set
    }
    let len_size = 1usize << (flags & 0x03);
    let name_len = c.uint(len_size)? as usize;
    // A sane dataset name. Anything else means we've walked off the end of the real links.
    if name_len == 0 || name_len > 1024 {
        return Err(Error::Unsupported("implausible link name length".into()));
    }
    let name = String::from_utf8_lossy(c.bytes(name_len)?).to_string();
    if link_type != 0 {
        // Soft and external links point at paths, not addresses; nothing here uses them.
        return Err(Error::Unsupported(format!("link type {link_type}")));
    }
    let addr = c.u64()?;
    Ok((name, addr, c.p))
}

/// Walk a dense attribute heap the same way we walk group links: direct blocks, messages laid end
/// to end. Failures are silent — a missing `scale_factor` degrades to unscaled data, which the
/// golden tests would catch, whereas refusing to open the file would lose everything.
pub(crate) fn dense_attributes(d: &[u8], heap_addr: u64) -> Vec<(String, Option<crate::Value>)> {
    let mut out = Vec::new();
    let Ok(blocks) = direct_blocks(d, heap_addr) else {
        return out;
    };
    for (start, end) in blocks {
        // ponytail: resynchronizing scan rather than a strict end-to-end walk. Dense attribute
        // records are *mostly* packed, but the heap leaves gaps we'd have to read its free-space
        // manager to predict — and one bad length would silently cost every attribute after it.
        // Trying each offset and skipping ahead on a hit costs nothing on a 512-byte block and
        // can't desync. The golden tests assert the values we actually need come out right.
        let mut p = start;
        while p < end {
            match crate::header::parse_attribute(d, p) {
                Ok((name, value, next))
                    if next > p
                        && next <= end
                        && plausible_name(&name)
                        && !out
                            .iter()
                            .any(|(n, _): &(String, Option<crate::Value>)| *n == name) =>
                {
                    out.push((name, value));
                    p = next;
                }
                _ => p += 1,
            }
        }
    }
    out
}

/// An attribute name a netCDF writer would actually emit — the guard that stops the scan from
/// accepting random bytes as a record.
fn plausible_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
}
