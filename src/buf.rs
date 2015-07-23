use std::ptr::copy_nonoverlapping;
use std::ops::{Index, RangeFrom, RangeTo, RangeFull};
use std::cmp::min;
use std::io::{Read, Write, Result};


const READ_MIN: u32 = 4096;
const ALLOC_MIN: usize = 16384;

/// Maximum size of buffer allowed.
/// Note: we assert on this size. Most network servers should set their own
/// limits to something much smaller.
pub const MAX_BUF_SIZE: usize = (1 << 32) - 2;

///
/// A buffer object to be used for reading from network
///
/// Assumptions:
///
/// 1. Buffer need to be growable as sometimes requests are large
/// 2. Buffer should deallocate when empty as
///    most of the time connections are idle
/// 2a. Deallocations are cheap as we have cool memory allocator (jemalloc)
/// 2b. First allocation should be big (i.e. kilobytes) not few bytes
/// 3. Should be easy too peek and get a slice as it makes packet parsing easy
/// 4. Cheap removing bytes at the start of the buf
/// 5. Buf consumes same size as vec
/// 6. Buf holds upto 4Gb of memory, larger network buffers are impractical for
///    most use cases
///
pub struct Buf {
    data: Option<Box<[u8]>>,
    consumed: u32,
    remaining: u32,
}

impl Buf {
    pub fn new() -> Buf {
        Buf {
            data: None,
            consumed: 0,
            remaining: 0,
        }
    }
    pub fn set_capacity(&mut self, bytes: usize) {
        self.data = self.data.take().map(|slice| {
            let old_len = slice.len();

            if self.consumed > 0 { // let's allocate new slice and move
                let mut vec = Vec::with_capacity(bytes);
                let cap = vec.capacity();
                unsafe {
                    vec.set_len(cap);
                    copy_nonoverlapping(
                        slice[self.consumed as usize..].as_ptr(),
                        vec[..].as_mut_ptr(),
                        slice.len() - self.consumed as usize
                                    - self.remaining as usize);
                }
                self.remaining = self.remaining
                    .saturating_add((cap - old_len) as u32)
                    .saturating_add(self.consumed);
                self.consumed = 0;
                Some(vec.into_boxed_slice())
            } else { // just reallocate
                let mut vec = slice.into_vec();
                let todo = bytes - vec.len();
                vec.reserve(todo);
                let cap = vec.capacity();
                unsafe { vec.set_len(cap) };
                self.remaining = self.remaining
                    .saturating_add((cap - old_len) as u32);
                Some(vec.into_boxed_slice())
            }
        }).unwrap_or_else(|| {
            let mut vec = Vec::with_capacity(bytes);
            let cap = vec.capacity();
            unsafe { vec.set_len(cap) };

            self.remaining = cap as u32;
            Some(vec.into_boxed_slice())
        })
    }
    fn remaining(&self) -> usize {
        self.remaining as usize
    }
    fn consumed(&self) -> usize {
        self.consumed as usize
    }
    pub fn capacity(&self) -> usize {
        self.data.as_ref().map(|x| x.len()).unwrap_or(0)
    }
    pub fn len(&self) -> usize {
        self.data.as_ref()
        .map(|x| x.len() - self.consumed() - self.remaining())
        .unwrap_or(0)
    }
    fn future_slice<'x>(&'x mut self) -> &'x mut [u8] {
        let rem = self.remaining();
        self.data.as_mut()
        .map(|x| {
            let upto = x.len();
            &mut x[upto - rem .. upto]
        })
        .unwrap()
    }
}

pub trait ReadBuf {
    fn read_to(&mut self, buf: &mut Buf) -> Result<()>;
    fn read_to_max(&mut self, buf: &mut Buf, max: usize) -> Result<bool>;
}

pub trait WriteBuf {
    fn write_from(&mut self, buf: &mut Buf);
}

impl<R:Read> ReadBuf for R {

    /// Read some bytes into buffer
    fn read_to(&mut self, buf: &mut Buf) -> Result<()> {
        if buf.remaining < READ_MIN {
            let ncap = buf.data.as_ref()
                         .map(|x| min(x.len()*2, MAX_BUF_SIZE))
                         .unwrap_or(ALLOC_MIN);
            buf.set_capacity(ncap);
        }
        let bytes = try!(self.read(buf.future_slice()));
        debug_assert!(bytes <= buf.remaining());
        buf.remaining -= bytes as u32;
        Ok(())
    }

    /// Reads no more than max bytes into vector and returns boolean flag
    /// of whether max bytes are reached
    fn read_to_max(&mut self, buf: &mut Buf, max: usize) -> Result<bool> {
        assert!(max < MAX_BUF_SIZE);
        let todo = max.saturating_sub(buf.len());
        if todo == 0 {
            return Ok(true);
        }
        if buf.remaining() < todo {
            buf.set_capacity(max);
        }
        let bytes = try!(self.read(&mut buf.future_slice()[..todo]));
        debug_assert!(bytes < buf.remaining());
        buf.remaining -= bytes as u32;
        Ok(buf.len() >= max)
    }
}

impl Into<Vec<u8>> for Buf {
    fn into(mut self) -> Vec<u8> {
        if self.consumed == 0 {
            self.data.take().map(|slice| {
                let mut vec = slice.into_vec();
                if self.remaining > 0 {
                    let nlen = vec.len() - self.remaining();
                    unsafe { vec.set_len(nlen) };
                }
                vec
            }).unwrap_or(Vec::new())
        } else {
            self[..].to_vec()
        }
    }
}

impl Index<RangeFull> for Buf {
    type Output = [u8];
    fn index<'x>(&'x self, _idx: RangeFull) -> &'x[u8] {
        self.data.as_ref()
        .map(|x| &x[self.consumed()..x.len() - self.remaining()])
        .unwrap_or(b"")
    }
}

impl Index<RangeTo<usize>> for Buf {
    type Output = [u8];
    fn index<'x>(&'x self, slice: RangeTo<usize>) -> &'x[u8] {
        let idx = slice.end;
        if idx == 0 {
            return b"";
        }
        assert!(idx <= self.len());
        &self.data.as_ref().unwrap()[self.consumed()..self.consumed() + idx]
    }
}

impl Index<RangeFrom<usize>> for Buf {
    type Output = [u8];
    fn index<'x>(&'x self, slice: RangeFrom<usize>) -> &'x[u8] {
        let idx = slice.start;
        if idx == self.len() {
            return b"";
        }
        assert!(idx <= self.len());
        let buf = &self.data.as_ref().unwrap();
        &buf[self.consumed() + idx .. buf.len() - self.remaining()]
    }
}

#[cfg(test)]
mod test {
    use super::Buf;
    use super::ReadBuf;
    use super::ALLOC_MIN;
    use mockstream::SharedMockStream;
    use std::iter::repeat;

    #[test]
    fn empty() {
        let buf = Buf::new();
        assert_eq!(&buf[..], b"");
    }

    #[test]
    fn read_from() {
        let mut s = SharedMockStream::new();
        s.push_bytes_to_read(b"hello");
        let mut buf = Buf::new();
        s.read_to(&mut buf).unwrap();
        assert_eq!(&buf[..], b"hello");
    }

    #[test]
    fn two_reads() {
        let mut s = SharedMockStream::new();
        s.push_bytes_to_read(b"hello");
        let mut buf = Buf::new();
        s.read_to(&mut buf).unwrap();
        s.push_bytes_to_read(b" world");
        s.read_to(&mut buf).unwrap();
        assert_eq!(&buf[..], b"hello world");
        assert_eq!(buf.capacity(), ALLOC_MIN);
    }

    #[test]
    fn realloc() {
        let mut s = SharedMockStream::new();
        s.push_bytes_to_read(b"hello");
        let mut buf = Buf::new();
        s.read_to(&mut buf).unwrap();
        s.push_bytes_to_read(&b"abcdefg".iter().cloned().cycle().take(1024*1024)
            .collect::<Vec<_>>()[..]);
        s.read_to(&mut buf).unwrap();
        assert_eq!(&buf[..9], b"helloabcd");
        assert_eq!(buf.len(), ALLOC_MIN);
        assert_eq!(buf.capacity(), ALLOC_MIN);
        s.read_to(&mut buf).unwrap();
        assert_eq!(buf.len(), 32768);
        assert_eq!(buf.capacity(), 32768);
        s.read_to(&mut buf).unwrap();
        assert_eq!(buf.len(), 65536);
        assert_eq!(buf.capacity(), 65536);
        s.read_to(&mut buf).unwrap();
        assert_eq!(buf.len(), 128 << 10);
        assert_eq!(buf.capacity(), 128 << 10);
        s.read_to(&mut buf).unwrap();
        assert_eq!(buf.len(), 256 << 10);
        assert_eq!(buf.capacity(), 256 << 10);
        s.read_to(&mut buf).unwrap();
        assert_eq!(buf.len(), 512 << 10);
        assert_eq!(buf.capacity(), 512 << 10);
        s.read_to(&mut buf).unwrap();
        assert_eq!(buf.len(), 1024 << 10);
        assert_eq!(buf.capacity(), 1024 << 10);
        s.read_to(&mut buf).unwrap();
        assert_eq!(buf.len(), 1048576 + 5);
        assert_eq!(buf.capacity(), 2048 << 10);
        assert_eq!(&buf[buf.len()-10..], b"bcdefgabcd");
    }
}
