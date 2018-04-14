//! # Netbuf
//!
//! [Documentation](https://docs.rs/netbuf) |
//! [Github](https://github.com/tailhook/netbuf) |
//! [Crate](https://crates.io/crates/netbuf)
//!
//! This module currently includes single `Buf` struct for holding buffers.
//! Comparing to `Vec` class buffer has different allocation policy and has
//! a marker of consumed data (i.e. already processed by protocol parser or
//! already written to socket)
//!
//! The `Buf` is deemed good both for input and output network buffer.
//!
//! It also contains helper methods `read_from` and `write_to` which are used
//! to read and append bytes from stream that implements Read and write bytes
//! from buffer to a stream which implements Write respectively.
//!
//! Note there are basically three ways to fill the buffer:
//!
//! * `Buf::read_from` -- preallocates some chunk and gives it to object
//!   implemeting Read
//! * `Write::write` -- writes chunk to buffer assuming more data will follow
//!   shortly, i.e. it does large preallocations
//! * `Buf::extend` -- writes chunk to buffer assuming it will not grow in the
//!   near perspective, so it allocates minimum chunk to hold the data
//!
//! In other words you should use:
//!
//! * `Buf::read_from` -- to read from the network
//! * `Write::write` -- when you are constructing object directly to the buffer
//!   incrementally
//! * `Buf::extend` -- when you put whole object in place and give it to the
//!   network code for sending
//!
//! More documentation is found in `Buf` object itself

#[cfg(test)] extern crate mockstream;

mod buf;
mod range;

pub use buf::Buf;
pub use range::RangeArgument;
