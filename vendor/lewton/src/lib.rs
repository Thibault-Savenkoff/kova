// Vorbis decoder written in Rust
//
// Copyright (c) 2016 est31 <MTest31@outlook.com>
// and contributors. All rights reserved.
// Licensed under MIT license, or Apache 2 license,
// at your option. Please see the LICENSE file
// attached to this source distribution for details.

#![no_std]
#![allow(warnings)]


#[macro_use]
extern crate alloc;
extern crate libm;
mod shim;
/// What the code takes from `std`, for the `use ::std::...` lines to go on working.
mod std {
	pub use core::{result, convert, fmt, cmp, iter, f32, mem, ops, option, slice, marker, clone, default};
	pub mod error { pub trait Error: core::fmt::Debug + core::fmt::Display {} }
}

macro_rules! try {
	($expr:expr) => (match $expr {
		$crate::std::result::Result::Ok(val) => val,
		$crate::std::result::Result::Err(err) => {
			return Err($crate::std::convert::From::from(err));
		}
	})
}

/*
// This little thing is very useful.
macro_rules! try {
	($expr:expr) => (match $expr {
		$crate::std::result::Result::Ok(val) => val,
		$crate::std::result::Result::Err(err) => {
			panic!("Panic on Err turned on for debug reasons. Encountered Err: {:?}", err)
		}
	})
}
// */

// The following macros are super useful for debugging

macro_rules! record_residue_pre_inverse {
	($residue_vectors:expr) => {
// 		for v in $residue_vectors.iter() {
// 			for &re in v {
// 				println!("{}", re);
// 			}
// 		}
	}
}

macro_rules! record_residue_post_inverse {
	($residue_vectors:expr) => {
// 		for v in $residue_vectors.iter() {
// 			for &re in v {
// 				println!("{}", re);
// 			}
// 		}
	}
}

macro_rules! record_pre_mdct {
	($audio_spectri:expr) => {
// 		for v in $audio_spectri.iter() {
// 			for &s in v {
// 				println!("{:.5}", s);
// 			}
// 		}
	}
}

macro_rules! record_post_mdct {
	($audio_spectri:expr) => {
// 		for v in $audio_spectri.iter() {
// 			for &s in v {
// 				println!("{:.4}", s);
// 			}
// 		}
	}
}

pub mod header;
mod header_cached;
mod huffman_tree;
mod imdct;
pub mod audio;
mod bitpacking;
pub mod samples;

#[cfg(feature = "ogg")]
#[doc(no_inline)]
pub use ogg::OggReadError;



/// Errors that can occur during decoding
#[derive(Debug)]
pub enum VorbisError {
	BadAudio(audio::AudioReadError),
	BadHeader(header::HeaderReadError),
	#[cfg(feature = "ogg")]
	OggError(OggReadError),
}

impl ::std::error::Error for VorbisError {}

impl ::std::fmt::Display for VorbisError {
	fn fmt(&self, fmt: &mut ::std::fmt::Formatter) -> Result<(), ::std::fmt::Error> {
		write!(fmt, "{}", match self {
			VorbisError::BadAudio(_) => "Vorbis bitstream audio decode problem",
			VorbisError::BadHeader(_) => "Vorbis bitstream header decode problem",
			#[cfg(feature = "ogg")]
			VorbisError::OggError(_) => "Ogg decode problem",
		})
	}
}

impl From<audio::AudioReadError> for VorbisError {
	fn from(err :audio::AudioReadError) -> VorbisError {
		VorbisError::BadAudio(err)
	}
}

impl From<header::HeaderReadError> for VorbisError {
	fn from(err :header::HeaderReadError) -> VorbisError {
		VorbisError::BadHeader(err)
	}
}

#[cfg(feature = "ogg")]
impl From<OggReadError> for VorbisError {
	fn from(err :OggReadError) -> VorbisError {
		VorbisError::OggError(err)
	}
}

fn ilog(val :u64) -> u8 {
	64 - val.leading_zeros() as u8
}

#[test]
fn test_ilog() {
	// Uses the test vectors from the Vorbis I spec
	assert_eq!(ilog(0), 0);
	assert_eq!(ilog(1), 1);
	assert_eq!(ilog(2), 2);
	assert_eq!(ilog(3), 2);
	assert_eq!(ilog(4), 3);
	assert_eq!(ilog(7), 3);
}

fn bit_reverse(n :u32) -> u32 {
	// From the stb_vorbis implementation
	let mut nn = n;
	nn = ((nn & 0xAAAAAAAA) >> 1) | ((nn & 0x55555555) << 1);
	nn = ((nn & 0xCCCCCCCC) >> 2) | ((nn & 0x33333333) << 2);
	nn = ((nn & 0xF0F0F0F0) >> 4) | ((nn & 0x0F0F0F0F) << 4);
	nn = ((nn & 0xFF00FF00) >> 8) | ((nn & 0x00FF00FF) << 8);
	return (nn >> 16) | (nn << 16);
}




