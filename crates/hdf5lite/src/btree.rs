//! The version-1 B-tree that indexes a chunked dataset's chunks.
//!
//! Layout version 3 always points at one of these. Each leaf key gives a chunk's stored size, a
//! filter mask and its offset in the dataset's coordinate space; the address that follows is where
//! the chunk's bytes live.

use crate::read::Cur;
use crate::{Error, Result};

/// One chunk's location on disk.
pub(crate) struct Chunk {
    pub addr: u64,
    /// Bytes on disk — after filtering, so usually smaller than the chunk's logical size.
    pub size: u32,
    /// Chunk origin in elements, one entry per dataset dimension.
    pub offset: Vec<u64>,
}

/// Collect every chunk the tree indexes. `rank` is the dataset's dimensionality.
pub(crate) fn chunks(d: &[u8], root: u64, rank: usize) -> Result<Vec<Chunk>> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    let mut guard = 0;
    while let Some(addr) = stack.pop() {
        guard += 1;
        // A malformed tree must not spin forever; real granules have a handful of nodes.
        if guard > 4096 {
            return Err(Error::Unsupported("chunk B-tree with >4096 nodes".into()));
        }
        if addr == u64::MAX {
            continue;
        }
        let mut c = Cur::new(d, addr as usize, "chunk B-tree node");
        c.sig(b"TREE", "B-tree")?;
        let node_type = c.u8()?;
        if node_type != 1 {
            return Err(Error::Unsupported(format!("B-tree node type {node_type}")));
        }
        let level = c.u8()?;
        let entries = c.u16()? as usize;
        c.skip(8 * 2)?; // left and right sibling addresses

        // Keys and child pointers alternate: K0 C0 K1 C1 … Kn. Each key is the chunk's size,
        // filter mask, and (rank + 1) offsets — the trailing one is the element offset, always 0.
        for _ in 0..entries {
            let size = c.u32()?;
            let _filter_mask = c.u32()?;
            let mut offset = Vec::with_capacity(rank);
            for _ in 0..rank {
                offset.push(c.u64()?);
            }
            c.skip(8)?; // the extra element-index dimension
            let child = c.u64()?;
            if level == 0 {
                out.push(Chunk {
                    addr: child,
                    size,
                    offset,
                });
            } else {
                stack.push(child);
            }
        }
        // The final key bounds the last entry and has no child.
    }
    Ok(out)
}
