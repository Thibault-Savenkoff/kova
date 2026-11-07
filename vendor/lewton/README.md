# lewton, as Kova uses it

[lewton](https://crates.io/crates/lewton) 0.10.2 by est31 and contributors, MIT OR Apache-2.0 (`LICENSE`): a pure-Rust
Ogg Vorbis decoder. Kova's runtime has no `std`, so this copy is changed:

- only the packet decoder is kept (`header`, `audio`, `imdct`, `huffman_tree`, `bitpacking`, `header_cached`,
  `samples`): the Ogg reader, the C API, the comment header (and with it `byteorder`) and the debug printing are
  removed, and Kova reads the Ogg pages itself (`shared::ogg_packets`);
- `#![no_std]` + `alloc`, with a small `std` module for the `use std::...` lines; `src/shim.rs` gives back the
  float methods (`sin`, `cos`, `exp`...) through `libm`;
- `tinyvec::TinyVec` is a `Vec`.
