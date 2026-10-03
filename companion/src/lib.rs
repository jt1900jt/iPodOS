//! Library database and art pack builder. The core (build/write/read/art/tags) has no
//! filesystem dependency so it can be compiled to wasm for the browser companion.

pub mod art;
pub mod build;
pub mod font;
pub mod journal;
pub mod format;
pub mod model;
pub mod read;
pub mod sortkey;
pub mod tags;
pub mod write;

#[cfg(feature = "fs")]
pub mod scan;
