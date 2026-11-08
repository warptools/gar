//! Fetching from a remote heap: the transports that read files out of one, by heap-relative path,
//! and `fetch_tree`, which pulls a tree through one into the local heap, verifying everything.
//!
//! A remote heap is named by its base: the directory (or URL) that directly contains `blobcas/` and `treeidx/`.
//! Transports only move bytes; they verify nothing.
//! Everything read through one is untrusted until the caller has hashed it.

mod fetch_error;
mod fetch_tree;
mod http;
mod local;
mod remote;
mod transport;

#[cfg(test)]
mod http_test;
#[cfg(test)]
mod local_test;
#[cfg(test)]
mod remote_test;

pub use fetch_error::{FetchError, Rejection};
pub use fetch_tree::{Outcome, fetch_tree};
pub use http::HttpTransport;
pub use local::LocalTransport;
pub use remote::Remote;
pub use transport::{OpenError, Transport};
