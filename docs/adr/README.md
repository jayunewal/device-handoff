# Architecture decision records

Short notes on decisions that are expensive to reverse, written when the decision is made. A decision can be superseded by a later record, but old records are not rewritten.

| Record | Decision |
| --- | --- |
| [0001](0001-record-architecture-decisions.md) | Keep architecture decision records |
| [0002](0002-rust-core.md) | A shared Rust core with a Flutter UI |
| [0003](0003-iroh-transport.md) | iroh for connections between devices |
| [0004](0004-apache-2-license.md) | Apache-2.0, and no code copied from copyleft projects |
| [0005](0005-cbor-wire-format.md) | CBOR in length-prefixed frames for control messages |
| [0006](0006-pairing-admission-and-requests.md) | Who is let in, and how requests are handled |
| [0007](0007-quic-windows-and-connection-memory.md) | QUIC flow-control windows, and how much a peer can make a device hold |
| [0008](0008-file-transfer.md) | File transfer: one offer, lanes of their own, hashing while sending |
| [0009](0009-window-streaming.md) | Window streaming: the Mac offers, late frames are dropped, input stays in the window |
