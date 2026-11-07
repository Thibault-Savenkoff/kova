# rusty_aac, as Kova uses it

[rusty_aac](https://crates.io/crates/rusty_aac) 2.0.1 by Mata Network, Apache-2.0 (`LICENSE`): a pure-Rust AAC
decoder (LC, HE-AAC v1/v2, ...). Kova's runtime has no `std`, and the crate needs it, so this copy is changed:

- the encoder, the `lab` module and the doctests are removed (only decoding is wanted);
- `#![no_std]` + `alloc`; `src/shim.rs` gives back the float methods (`sin`, `powf`, `round_ties_even`...) through
  `libm`, and a spin-flag `OnceLock`;
- inner `#![warn(...)]` lint attributes are removed.

Checked against FFmpeg: an ADTS file decodes to the same samples (error 1e-12).
