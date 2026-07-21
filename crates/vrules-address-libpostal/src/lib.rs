//! libpostal-backed address analyzer for vrules.
//!
//! The default crate build is link-free so the workspace remains buildable on
//! machines without libpostal. Enable `native` to link against a built/installed
//! libpostal C library.

#[cfg(feature = "native")]
mod native;

#[cfg(feature = "native")]
pub use native::LibpostalAnalyzer;
