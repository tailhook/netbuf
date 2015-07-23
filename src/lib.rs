#[cfg(test)] extern crate mockstream;

mod buf;

pub use buf::Buf;
pub use buf::ReadBuf;
pub use buf::WriteBuf;
pub use buf::MAX_BUF_SIZE;
