//! Primitive aliases matching the types referenced in the ICD.
//!
//! These types provide big-endian wrappers around standard Rust primitives,
//! matching the naming conventions used in the NEXRAD ICD documentation.

use std::cmp::Ordering;
use std::fmt;
use std::ops::{BitAnd, Shr};
use zerocopy::{big_endian, FromBytes, Immutable, KnownLayout};

/// 8-bit code value (Code1 in ICD).
pub type Code1 = u8;
/// 8-bit unsigned integer (Integer1 in ICD).
pub type Integer1 = u8;
/// 8-bit scaled unsigned integer (ScaledInteger1 in ICD).
pub type ScaledInteger1 = u8;

/// Defines a big-endian wrapper type with standard trait implementations.
///
/// Generates: struct definition, `new`/`get` methods, `Debug`, `Display`,
/// `PartialEq<native>`, `PartialOrd<native>`, and `From<native>`.
macro_rules! define_be_wrapper {
    (
        $(#[$meta:meta])*
        $name:ident($inner:ty) => $native:ty
        $(, derive($($extra:ident),+))?
    ) => {
        $(#[$meta])*
        #[repr(transparent)]
        #[derive(Clone, Copy, PartialEq, FromBytes, Immutable, KnownLayout $($(, $extra)+)?)]
        pub struct $name($inner);

        impl $name {
            #[doc = concat!("Creates a new `", stringify!($name), "` from a native `", stringify!($native), "` value.")]
            pub fn new(value: $native) -> Self {
                Self(<$inner>::new(value))
            }

            #[doc = concat!("Returns the value as a native `", stringify!($native), "`.")]
            pub fn get(self) -> $native {
                self.0.get()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.get().fmt(f)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.get().fmt(f)
            }
        }

        impl PartialEq<$native> for $name {
            fn eq(&self, other: &$native) -> bool {
                self.0.get() == *other
            }
        }

        impl PartialOrd<$native> for $name {
            fn partial_cmp(&self, other: &$native) -> Option<Ordering> {
                self.0.get().partial_cmp(other)
            }
        }

        impl From<$native> for $name {
            fn from(value: $native) -> Self {
                Self::new(value)
            }
        }
    };
}

define_be_wrapper! {
    /// Big-endian unsigned 16-bit integer (Code2 in ICD).
    Code2(big_endian::U16) => u16, derive(Eq, Hash)
}

impl BitAnd<u16> for Code2 {
    type Output = u16;
    fn bitand(self, rhs: u16) -> u16 {
        self.0.get() & rhs
    }
}

impl Shr<i32> for Code2 {
    type Output = u16;
    fn shr(self, rhs: i32) -> u16 {
        self.0.get() >> rhs
    }
}

define_be_wrapper! {
    /// Big-endian unsigned 16-bit integer (Integer2 in ICD).
    Integer2(big_endian::U16) => u16, derive(Eq, Hash)
}

define_be_wrapper! {
    /// Big-endian unsigned 32-bit integer (Integer4 in ICD).
    Integer4(big_endian::U32) => u32, derive(Eq, Hash)
}

define_be_wrapper! {
    /// Big-endian 32-bit float (Real4 in ICD).
    Real4(big_endian::F32) => f32
}

define_be_wrapper! {
    /// Big-endian unsigned 16-bit integer (ScaledInteger2 in ICD).
    ScaledInteger2(big_endian::U16) => u16, derive(Eq, Hash)
}

define_be_wrapper! {
    /// Big-endian signed 16-bit integer (ScaledSInteger2 in ICD).
    ScaledSInteger2(big_endian::I16) => i16, derive(Eq, Hash)
}

define_be_wrapper! {
    /// Big-endian signed 16-bit integer (SInteger2 in ICD).
    SInteger2(big_endian::I16) => i16, derive(Eq, Hash)
}
