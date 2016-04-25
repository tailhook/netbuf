use std::ptr::{copy_nonoverlapping, copy};
use std::ops::{Index, IndexMut, RangeFrom, RangeTo, RangeFull, Range};
use std::cmp::{min, max};
use std::io::{Read, Write, Result};
use std::fmt::{self, Debug};

use range::RangeArgument;


const READ_MIN: usize = 4096;
const ALLOC_MIN: usize = 16384;

/// Maximum size of buffer allowed.
/// Note: we assert on this size. Most network servers should set their own
/// limits to something much smaller.
pub const MAX_BUF_SIZE: usize = 4294967294; // (1 << 32) - 2;

///
/// A buffer object to be used for reading from network
///
/// Assumptions:
///
/// 1. Buffer need to be growable as sometimes requests are large
///
/// 2. Buffer should deallocate when empty as
///    most of the time connections are idle
///
///      a. Deallocations are cheap as we have cool memory allocator (jemalloc)
///
///      b. First allocation should be big (i.e. kilobytes not few bytes)
///
/// 3. Should be easy too peek and get a slice as it makes packet parsing easy
///
/// 4. Cheap removing bytes at the start of the buf
///
/// 5. Buf itself has same size as Vec
///
/// 6. Buf holds upto 4Gb of memory, larger network buffers are impractical for
///    most use cases
///
pub struct Buf {
    data: Option<Box<[u8]>>,
    consumed: u32,
    remaining: u32,
}


// TODO(tailhook) use std::slice::bytes::copy_memory;
fn copy_memory(src: &[u8], dst: &mut [u8]) {
    assert!(src.len() == dst.len());
    unsafe {
        copy_nonoverlapping(src.as_ptr(), dst.as_mut_ptr(), dst.len());
    }
}


impl Buf {
    /// Create empty buffer. It has no preallocated size. It's always have
    /// deallocated underlying memory chunk when there are no useful bytes
    /// in the buffer.
    pub fn new() -> Buf {
        Buf {
            data: None,
            consumed: 0,
            remaining: 0,
        }
    }
    fn reserve(&mut self, bytes: usize) {
        self.data = self.data.take().map(|slice| {
            let old_cap = slice.len();
            let old_bytes = old_cap - self.consumed() - self.remaining();

            if self.consumed() > 0 { // let's allocate new slice and move
                let min_size = old_bytes + bytes;
                assert!(min_size < MAX_BUF_SIZE);
                let mut vec = Vec::with_capacity(
                    max(min_size, min(old_cap*2, MAX_BUF_SIZE)));
                let cap = vec.capacity();
                unsafe { vec.set_len(cap) };
                copy_memory(&slice[self.consumed()..old_cap - self.remaining()],
                            &mut vec[..old_bytes]);
                self.remaining = (cap - old_bytes) as u32;
                self.consumed = 0;
                Some(vec.into_boxed_slice())
            } else { // just reallocate
                let mut vec = slice.into_vec();
                vec.reserve(bytes);
                let cap = vec.capacity();
                unsafe { vec.set_len(cap) };
                self.remaining = (cap - old_bytes) as u32;
                Some(vec.into_boxed_slice())
            }
        }).unwrap_or_else(|| {
            let mut vec = Vec::with_capacity(max(bytes, ALLOC_MIN));
            let cap = vec.capacity();
            unsafe { vec.set_len(cap) };

            self.remaining = cap as u32;
            Some(vec.into_boxed_slice())
        })
    }
    fn reserve_exact(&mut self, bytes: usize) {
        self.data = self.data.take().map(|slice| {
            let old_cap = slice.len();
            let old_bytes = old_cap - self.consumed() - self.remaining();

            if self.consumed() > 0 { // let's allocate new slice and move
                let size = old_bytes + bytes;
                assert!(size < MAX_BUF_SIZE);
                let mut vec = Vec::with_capacity(size);
                let cap = vec.capacity();
                unsafe { vec.set_len(cap) };
                copy_memory(&slice[self.consumed()..old_cap - self.remaining()],
                            &mut vec[..old_bytes]);
                self.remaining = (cap - old_bytes) as u32;
                self.consumed = 0;
                Some(vec.into_boxed_slice())
            } else { // just reallocate
                let mut vec = slice.into_vec();
                vec.reserve_exact(bytes);
                let cap = vec.capacity();
                unsafe { vec.set_len(cap) };
                self.remaining = (cap - old_bytes) as u32;
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

    /// Mark the first `bytes` of the buffer as read. Basically it's shaving
    /// off bytes from the buffer. But does it effeciently. When there are
    /// no more bytes in the buffer it's deallocated.
    ///
    /// Note: Buffer currently don't shrink on calling this method. It's
    /// assumed that all bytes will be consumed shortly. In case you're
    /// appending to the buffer after consume, old data is discarded.
    ///
    /// # Panics
    ///
    /// Panics if `bytes` is larger than current length of buffer
    pub fn consume(&mut self, bytes: usize) {
        let ln = self.len();
        assert!(bytes <= ln);
        if bytes == ln {
            *self = Buf::new();
        } else {
            self.consumed += bytes as u32;
        }
    }

    /// Allows to remove arbitrary range of bytes
    ///
    /// A more comprehensive version of `consume()`. It's occasionaly useful
    /// if you data by frames/chunks but want to buffer the whole body anyway.
    /// E.g. in http chunked encoding you have each chunk prefixed by it's
    /// length, but it doesn't mean you can't buffer the whole request into
    /// the memory. This method allows to continue reading next chunk into
    /// the same buffer while removing chunk length.
    ///
    /// Note: it's not super efficient, as it requires to move(copy) bytes
    /// after the range, in case range is neither at the start nor at the
    /// end of buffer. Still it should be faster than copying everything
    /// to yet another buffer.
    ///
    /// We never shrink the buffer here (except when it becomes empty, to
    /// keep this invariant), assuming that you will receive more data into
    /// the buffer shortly.
    ///
    /// The `RangeArgument` type is a temporary type until rust provides
    /// the one in standard library, you should use the range syntax directly:
    /// ```
    ///     buf.remove_range(5..7)
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if range is invalid for the buffer
    pub fn remove_range<R: Into<RangeArgument>>(&mut self, range: R) {
        use range::RangeArgument::*;
        match range.into() {
            RangeTo(x) | Range(0, x) => self.consume(x),
            RangeFrom(0) => *self = Buf::new(),
            RangeFrom(x) => {
                let ln = self.len();
                assert!(x < ln);
                self.remaining += (ln - x) as u32;
            }
            Range(x, y) => {
                let ln = self.len();
                if x == y { return; }
                assert!(x < y);
                let removed_bytes = y - x;
                assert!(y < ln);
                let start = self.consumed() + x;
                let end = self.consumed() + y;
                if let Some(ref mut data) = self.data {
                    let dlen = data.len();
                    unsafe {
                        copy(data[end..].as_ptr(),
                            data[start..dlen - removed_bytes].as_mut_ptr(),
                            dlen - end);
                    }
                    self.remaining += removed_bytes as u32;
                } else {
                    panic!("Not-existent buffere where data exists");
                }
            }
        }
    }


    /// Capacity of the buffer. I.e. the bytes it is allocated for. Use for
    /// debugging or for calculating memory usage. Note it's not guaranteed
    /// that you can write `buf.capacity() - buf.len()` bytes without resize
    pub fn capacity(&self) -> usize {
        self.data.as_ref().map(|x| x.len()).unwrap_or(0)
    }

    /// Number of useful bytes in the buffer
    pub fn len(&self) -> usize {
        self.data.as_ref()
        .map(|x| x.len() - self.consumed() - self.remaining())
        .unwrap_or(0)
    }

    /// Is buffer is empty. Potentially a little bit faster than
    /// getting `len()`
    pub fn is_empty(&self) -> bool {
        self.data.is_none()
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

    /// Extend buffer. Note unlike `Write::write()` and `read_from()` this
    /// method reserves smallest possible chunk of memory. So it's inefficient
    /// to grow with this method.  You may use Write trait to grow
    /// incrementally.
    pub fn extend(&mut self, buf: &[u8]) {
        if self.remaining() < buf.len() {
            self.reserve_exact(buf.len());
        }
        copy_memory(buf, &mut self.future_slice()[..buf.len()]);
        self.remaining -= buf.len() as u32;
    }

    /// Read some bytes from stream (object implementing `Read`) into buffer
    ///
    /// Note this does *not* continue read until getting `WouldBlock`. It
    /// passes all errors as is. It preallocates some chunk to read into
    /// buffer, it may be possible that socket still has bytes buffered after
    /// this method returns. This method is expected either to be called until
    /// `WouldBlock` is returned or is used with level-triggered polling.
    pub fn read_from<R:Read>(&mut self, stream: &mut R) -> Result<usize> {
        if self.remaining() < READ_MIN {
            self.reserve(READ_MIN);
        }
        let bytes = try!(stream.read(self.future_slice()));
        debug_assert!(bytes <= self.remaining());
        self.remaining -= bytes as u32;
        Ok(bytes)
    }

    /// Reads no more than max bytes into buffer and returns boolean flag
    /// of whether max bytes are reached
    ///
    /// Except limit on number of bytes and slightly different allocation
    /// strategy this method has same consideration as read_from
    ///
    /// Note this method might be used for two purposes:
    ///
    /// 1. Limit number of bytes buffered until parser can process data
    ///   (for example HTTP header size, which is number of bytes read before
    ///   `\r\n\r\n` delimiter reached)
    /// 2. Wait until exact number of bytes fully received
    ///
    /// Since we support (1) we don't preallocate buffer for exact size of
    /// the `max` value. It also helps a little in case (2) for minimizing
    /// DDoS attack vector. If that doesn't suit you, you may with to use
    /// `Vec::with_capacity()` for the purpose of (2).
    ///
    /// On the countrary we don't overallocate more than `max` bytes, so if
    /// you expect data to go after exact number of bytes read. You might
    /// better use raw `read_from()` and check buffer length.
    ///
    pub fn read_max_from<R:Read>(&mut self, max: usize, stream: &mut R)
        -> Result<bool>
    {
        assert!(max < MAX_BUF_SIZE);
        let todo = max.saturating_sub(self.len());
        if todo == 0 {
            return Ok(true);
        }
        if self.remaining() < READ_MIN {
            if self.capacity() * 2 < max {
                // TODO is too large we may never need so big buffer
                self.reserve(READ_MIN);
            } else {
                self.reserve_exact(todo);
            }
        }
        let bytes = {
            let slc = self.future_slice();
            let do_now = min(slc.len(), todo);
            try!(stream.read(&mut slc[..do_now]))
        };
        debug_assert!(bytes <= self.remaining());
        self.remaining -= bytes as u32;
        Ok(self.len() >= max)
    }

    /// Write contents of buffer to the stream (object implementing
    /// the Write trait). We assume that stream is non-blocking, use
    /// `Write::write` (instead of `Write::write_all`) and return all errors
    /// to the caller (including `WouldBlock` or `Interrupted`).
    ///
    /// Instead of returning number of bytes method `consume()`s bytes from
    /// buffer, so it's safe to retry calling the method at any moment. Also
    /// it's common pattern to append more data to the buffer between calls.
    pub fn write_to<W:Write>(&mut self, sock: &mut W) -> Result<usize> {
        let bytes = match sock.write(&self[..]) {
            Ok(bytes) => bytes,
            Err(e) => return Err(e),
        };
        self.consume(bytes);
        Ok(bytes)
    }
}


impl Debug for Buf {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Buf {{ len={}; consumed={}; remaining={} }}",
               self.len(), self.consumed(), self.remaining())
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

impl Index<usize> for Buf {
    type Output = u8;
    fn index<'x>(&'x self, index: usize) -> &'x u8 {
        if let Some(ref data) = self.data {
            return &data[self.consumed() + index]
        }
        panic!("cannot index empty buffer");

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

impl Index<Range<usize>> for Buf {
    type Output = [u8];
    fn index<'x>(&'x self, slice: Range<usize>) -> &'x[u8] {
        let start = slice.start;
        let end = slice.end;
        if end == 0 {
            return b"";
        }
        assert!(end <= self.len());
        assert!(start <= end);
        let buf = &self.data.as_ref().unwrap();
        &buf[self.consumed() + start .. self.consumed() + end]
    }
}

impl IndexMut<usize> for Buf {
    fn index_mut<'x>(&'x mut self, index: usize) -> &'x mut u8 {
        let consumed = self.consumed();
        if let Some(ref mut data) = self.data {
            return &mut data[consumed + index]
        }
        panic!("cannot index empty buffer");

    }
}

impl IndexMut<RangeFull> for Buf {
    fn index_mut<'x>(&'x mut self, _idx: RangeFull) -> &'x mut[u8] {
        let consumed = self.consumed();
        let remaining = self.remaining();
        self.data.as_mut()
        .map(|x| {
            let len = x.len();
            &mut x[consumed..len - remaining]
        })
        .unwrap_or(&mut [])
    }
}

impl IndexMut<RangeTo<usize>> for Buf {
    fn index_mut<'x>(&'x mut self, slice: RangeTo<usize>) -> &'x mut[u8] {
        let idx = slice.end;
        if idx == 0 {
            return &mut [];
        }
        assert!(idx <= self.len());
        let consumed = self.consumed();
        &mut self.data.as_mut().unwrap()[consumed..consumed + idx]
    }
}

impl IndexMut<RangeFrom<usize>> for Buf {
    fn index_mut<'x>(&'x mut self, slice: RangeFrom<usize>) -> &'x mut[u8] {
        let idx = slice.start;
        if idx == self.len() {
            return &mut [];
        }
        assert!(idx <= self.len());
        let consumed = self.consumed();
        let remaining = self.remaining();
        let mut buf = self.data.as_mut().unwrap();
        let len = buf.len();
        &mut buf[consumed + idx .. len - remaining]
    }
}

impl IndexMut<Range<usize>> for Buf {
    fn index_mut<'x>(&'x mut self, slice: Range<usize>) -> &'x mut[u8] {
        let start = slice.start;
        let end = slice.end;
        if end == 0 {
            return &mut [];
        }
        assert!(end <= self.len());
        assert!(start <= end);
        let consumed = self.consumed();
        let buf = self.data.as_mut().unwrap();
        &mut buf[consumed + start .. consumed + end]
    }
}

impl Write for Buf {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        if self.remaining() < buf.len() {
            self.reserve(buf.len());
        }
        copy_memory(buf, &mut self.future_slice()[..buf.len()]);
        self.remaining -= buf.len() as u32;
        Ok(buf.len())
    }
    fn flush(&mut self) -> Result<()> { Ok(()) }
}

#[cfg(test)]
mod test {
    use std::io::Write;
    use super::Buf;
    use super::ALLOC_MIN;
    use mockstream::SharedMockStream;

    #[test]
    fn empty() {
        let buf = Buf::new();
        assert_eq!(&buf[..], b"");
        assert_eq!(format!("{:?}", buf), "Buf { len=0; consumed=0; remaining=0 }")
    }

    #[test]
    fn read_from() {
        let mut s = SharedMockStream::new();
        s.push_bytes_to_read(b"hello");
        let mut buf = Buf::new();
        assert_eq!(buf.read_from(&mut s).unwrap(), 5);
        assert_eq!(&buf[..], b"hello");
    }

    #[test]
    fn read_max_from() {
        let mut s = SharedMockStream::new();
        s.push_bytes_to_read(b"hello");
        let mut buf = Buf::new();
        s.push_bytes_to_read(b"hello world");
        assert!(!buf.read_max_from(1024*1024, &mut s).unwrap());
        assert_eq!(&buf[..], b"hellohello world");
        assert!(buf.capacity() < ALLOC_MIN*2);
        s.push_bytes_to_read(b" from me!Oh, crap!");
        assert!(buf.read_max_from(25, &mut s).unwrap());
        assert_eq!(&buf[..], b"hellohello world from me!");
        assert!(buf.capacity() < ALLOC_MIN*2);
        assert!(buf.read_max_from(25, &mut s).unwrap());
        assert_eq!(&buf[..], b"hellohello world from me!");
        assert!(buf.capacity() < ALLOC_MIN*2);
        buf.consume(5);
        assert!(buf.read_max_from(22, &mut s).unwrap());
        assert_eq!(&buf[..], b"hello world from me!Oh");
        assert!(buf.capacity() < ALLOC_MIN*2);
    }

    #[test]
    fn two_reads() {
        let mut s = SharedMockStream::new();
        s.push_bytes_to_read(b"hello");
        let mut buf = Buf::new();
        buf.read_from(&mut s).unwrap();
        s.push_bytes_to_read(b" world");
        buf.read_from(&mut s).unwrap();
        assert_eq!(&buf[..], b"hello world");
        assert_eq!(buf.capacity(), ALLOC_MIN);
    }

    #[test]
    fn realloc() {
        let mut s = SharedMockStream::new();
        s.push_bytes_to_read(b"hello");
        let mut buf = Buf::new();
        buf.read_from(&mut s).unwrap();
        s.push_bytes_to_read(&b"abcdefg".iter().cloned().cycle().take(1024*1024)
            .collect::<Vec<_>>()[..]);
        buf.read_from(&mut s).unwrap();
        assert_eq!(&buf[..9], b"helloabcd");
        assert_eq!(buf.len(), ALLOC_MIN);
        assert_eq!(buf.capacity(), ALLOC_MIN);
        buf.read_from(&mut s).unwrap();
        assert_eq!(buf.len(), buf.capacity());
        assert!(buf.capacity() >= 32768);
        println!("Capacity {}", buf.capacity());
        buf.read_from(&mut s).unwrap();
        println!("Capacity {}", buf.capacity());
        buf.read_from(&mut s).unwrap();
        println!("Capacity {}", buf.capacity());
        buf.read_from(&mut s).unwrap();
        println!("Capacity {}", buf.capacity());
        buf.read_from(&mut s).unwrap();
        println!("Capacity {}", buf.capacity());
        buf.read_from(&mut s).unwrap();
        println!("Capacity {}", buf.capacity());
        buf.read_from(&mut s).unwrap();
        println!("Capacity {}", buf.capacity());
        assert_eq!(buf.len(), 1048576 + 5);
        assert!(buf.capacity() >= buf.len());
        assert!(buf.capacity() <= 2100000);
        assert_eq!(&buf[buf.len()-10..], b"bcdefgabcd");
    }

    #[test]
    fn two_writes() {
        let mut buf = Buf::new();
        assert_eq!(buf.write(b"hello").unwrap(), 5);
        assert_eq!(buf.write(b" world").unwrap(), 6);
        assert_eq!(&buf[..], b"hello world");
        assert_eq!(buf.capacity(), ALLOC_MIN);
    }

    #[test]
    fn two_extends() {
        let mut buf = Buf::new();
        buf.extend(b"hello");
        buf.extend(b" world");
        assert_eq!(&buf[..], b"hello world");
        assert_eq!(buf.capacity(), 11);
    }

    #[test]
    fn write_extend() {
        let mut buf = Buf::new();
        buf.write(b"hello").unwrap();
        buf.extend(b" world");
        assert_eq!(&buf[..], b"hello world");
        assert_eq!(buf.capacity(), ALLOC_MIN);
    }

    #[test]
    fn extend_write() {
        let mut buf = Buf::new();
        buf.extend(b"hello");
        buf.write(b" world").unwrap();
        assert_eq!(&buf[..], b"hello world");
        let cap = buf.capacity();
        // We don't know exact allocation policy for vec
        // So this check is fuzzy
        assert!(cap >= 11);
        assert!(cap < 256);
    }

    #[test]
    fn into() {
        let mut buf = Buf::new();
        buf.extend(b"hello");
        buf.write(b" world").unwrap();
        let vec: Vec<u8> = buf.into();
        assert_eq!(&vec[..], b"hello world");
        let cap = vec.capacity();
        // We don't know exact allocation policy for vec
        // So this check is fuzzy
        assert!(cap >= 11);
        assert!(cap < 256);
    }

    #[test]
    fn consumed_into() {
        let mut buf = Buf::new();
        buf.extend(b"hello");
        buf.write(b" world").unwrap();
        buf.consume(6);
        let vec: Vec<u8> = buf.into();
        assert_eq!(&vec[..], b"world");
        assert_eq!(vec.capacity(), 5);
    }
    #[test]
    fn write_to() {
        let mut s = SharedMockStream::new();
        let mut buf = Buf::new();
        buf.extend(b"hello world");
        assert_eq!(buf.write_to(&mut s).unwrap(), 11);
        assert_eq!(&s.pop_bytes_written()[..], b"hello world");
    }

    #[test]
    fn extend_consume_extend() {
        let mut buf = Buf::new();
        buf.extend(b"hello");
        buf.consume(3);
        buf.extend(b" world");
        assert_eq!(&buf[..], b"lo world");
        assert_eq!(buf.capacity(), 8);
    }

    #[test]
    fn extend_consume_write() {
        let mut buf = Buf::new();
        buf.extend(b"hello");
        buf.consume(3);
        buf.write(b"Some larger string for you").unwrap();
        assert_eq!(&buf[..], b"loSome larger string for you");
        assert_eq!(buf.capacity(), 28);
    }

    #[test]
    fn consume_all() {
        let mut buf = Buf::new();
        buf.extend(b"hello");
        buf.write(b" world").unwrap();
        buf.consume(11);
        assert_eq!(buf.capacity(), 0);
        assert_eq!(&buf[..], b"");
        let vec: Vec<u8> = buf.into();
        assert_eq!(&vec[..], b"");
        assert_eq!(vec.capacity(), 0);
    }

    #[test]
    fn index() {
        let mut buf = Buf::new();
        buf.extend(b"Hello World!");
        assert_eq!(buf[0], b'H');
        assert_eq!(buf[6], b'W');
        buf.consume(2);
        assert_eq!(buf[2], b'o');
        assert_eq!(buf[8], b'd');
    }

    #[test]
    fn index_mut() {
        let mut buf = Buf::new();
        buf.extend(b"Hello World!");
        assert_eq!(buf[0], b'H');
        buf[0] = b'h';
        assert_eq!(buf[6], b'W');
        buf[6] = b'M';
        assert_eq!(&buf[..], b"hello Morld!");
        buf.consume(2);
        buf[1] = b'e';
        buf[5] = b'e';
        buf[8] = b'e';
        assert_eq!(&buf[..], b"leo Merle!");
    }

    #[test]
    fn ranges() {
        let mut buf = Buf::new();
        buf.extend(b"Hello world!");
        assert_eq!(&buf[..], b"Hello world!");
        assert_eq!(&buf[..0], b"");
        assert_eq!(&buf[..5], b"Hello");
        assert_eq!(&buf[..12], b"Hello world!");
        assert_eq!(&buf[..7], b"Hello w");
        assert_eq!(&buf[0..], b"Hello world!");
        assert_eq!(&buf[3..], b"lo world!");
        assert_eq!(&buf[6..], b"world!");
        assert_eq!(&buf[12..], b"");
        assert_eq!(&buf[0..0], b"");
        assert_eq!(&buf[2..8], b"llo wo");
        assert_eq!(&buf[0..12], b"Hello world!");
        assert_eq!(&buf[0..5], b"Hello");
        assert_eq!(&buf[7..12], b"orld!");
        assert_eq!(&buf[3..3], b"");
        assert_eq!(&buf[3..9], b"lo wor");
    }

    #[test]
    fn consume_ranges() {
        let mut buf = Buf::new();
        buf.extend(b"Crappy stuff");
        buf.extend(b"Hello world!");
        buf.consume(12);
        assert_eq!(&buf[..], b"Hello world!");
        assert_eq!(&buf[..0], b"");
        assert_eq!(&buf[..5], b"Hello");
        assert_eq!(&buf[..12], b"Hello world!");
        assert_eq!(&buf[..7], b"Hello w");
        assert_eq!(&buf[0..], b"Hello world!");
        assert_eq!(&buf[3..], b"lo world!");
        assert_eq!(&buf[6..], b"world!");
        assert_eq!(&buf[12..], b"");
        assert_eq!(&buf[2..8], b"llo wo");
        assert_eq!(&buf[0..12], b"Hello world!");
        assert_eq!(&buf[0..5], b"Hello");
        assert_eq!(&buf[7..12], b"orld!");
        assert_eq!(&buf[3..3], b"");
        assert_eq!(&buf[3..9], b"lo wor");
    }

    #[test]
    fn remove_ranges() {
        let mut buf = Buf::new();
        buf.extend(b"Crappy stuff");
        buf.extend(b"Hello world!");
        assert_eq!(&buf[..], b"Crappy stuffHello world!");
        buf.remove_range(..4);
        assert_eq!(&buf[..], b"py stuffHello world!");
        buf.remove_range(3..8);
        assert_eq!(&buf[..], b"py Hello world!");
        buf.remove_range(7..14);
        assert_eq!(&buf[..], b"py Hell!");
        buf.remove_range(7..);
        assert_eq!(&buf[..], b"py Hell");
    }

    #[test]
    fn mut_ranges() {
        let mut buf = Buf::new();
        buf.extend(b"Crappy stuff");
        buf.extend(b"Hello world!");
        buf.consume(12);
        buf[..].clone_from_slice(b"HELLO WORLD.");
        assert_eq!(&buf[..], b"HELLO WORLD.");
        buf[..5].clone_from_slice(b"hell!");
        assert_eq!(&buf[..], b"hell! WORLD.");
        buf[4..10].clone_from_slice(b"o worl");
        assert_eq!(&buf[..], b"hello worlD.");
        buf[10..].clone_from_slice(b"d!");
        assert_eq!(&buf[..], b"hello world!");
    }
}
