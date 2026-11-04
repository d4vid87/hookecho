//!
//! Message types 4 and 10 "Console Message" carry free-form text between the RDA and RPG. Type 4
//! originates from the RDA and type 10 originates from the RPG. The message contains a size field
//! followed by variable-length text of up to 404 bytes.
//!

mod message;
pub use message::Message;

pub(crate) mod raw;

#[cfg(test)]
mod snapshot_test;
