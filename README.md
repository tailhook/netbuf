Netbuf
======

[Documentation](https://docs.rs/netbuf) |
[Github](https://github.com/tailhook/netbuf) |
[Crate](https://crates.io/crates/netbuf)

The network buffer for usage in asynchronous network applications. Has right
interface for reading from network into the ``Buf`` and for parsing packets
directly from it. It also has capacity management tuned for network application
(i.e. it starts growing fast, but deallocated for idle connections).


License
=======

Licensed under either of

* Apache License, Version 2.0,
  (./LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license (./LICENSE-MIT or http://opensource.org/licenses/MIT)
  at your option.

Contribution
------------

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
