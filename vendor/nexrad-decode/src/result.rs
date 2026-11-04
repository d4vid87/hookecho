//! Result and error types for NEXRAD decoding operations.

use thiserror::Error as ThisError;

/// A specialized Result type for NEXRAD decoding operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur during NEXRAD data decoding.
#[derive(ThisError, Debug)]
pub enum Error {
    /// An I/O error occurred while reading data.
    #[error("data file IO error")]
    Io(#[from] std::io::Error),
    /// A general decoding error with a descriptive message.
    #[error("file decoding error: {0}")]
    Decoding(String),
    /// A message is missing required collection date/time fields.
    #[error("message is missing collection date/time")]
    MessageMissingDate,
    /// Reached end of input data unexpectedly during parsing.
    #[error("unexpected end of file to input data")]
    UnexpectedEof,
    /// Message length field is invalid for the message type.
    #[error("invalid message length for type {message_type}: cannot rewind {delta} bytes")]
    InvalidMessageLength {
        /// The message type being parsed.
        message_type: String,
        /// The invalid byte offset.
        delta: i32,
    },
    /// A data block pointer points outside valid message bounds.
    #[error("invalid data block pointer: cannot rewind {bytes} bytes at position {position}")]
    InvalidDataBlockPointer {
        /// Number of bytes the pointer tried to rewind.
        bytes: usize,
        /// Current position in the data.
        position: usize,
    },
    /// Encountered an unrecognized data block type identifier.
    #[error("unknown data block type: {block_type}")]
    UnknownDataBlockType {
        /// The unrecognized block type identifier.
        block_type: String,
    },
    /// A data structure spans across segment boundaries.
    #[error("data structure spans segment boundary at position {position}")]
    DataSpansSegmentBoundary {
        /// Position where the boundary was crossed.
        position: usize,
    },
}
