Netbuf
======

[Documentation](https://docs.rs/netbuf) |
[Github](https://github.com/tailhook/netbuf) |
[Crate](https://crates.io/crates/netbuf)

The network buffer for usage in asynchronous network applications. Has right
interface for reading from network into the ``Buf`` and for parsing packets
directly from it. It also has capacity management tuned for network application
(i.e. it starts growing fast, but deallocated for idle connections).

