======
Netbuf
======


:Status: beta
:Documentation: http://tailhook.github.io/netbuf/
:Travis:
    .. image:: https://travis-ci.org/tailhook/netbuf.svg?branch=master
        :target: https://travis-ci.org/tailhook/netbuf

The network buffer for usage in asynchronous network applications. Has right
interface for reading from network into the ``Buf`` and for parsing packets
directly from it. It also has capacity management tuned for network application
(i.e. it starts growing fast, but deallocated for idle connections).

