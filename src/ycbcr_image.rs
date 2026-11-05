// Copyright 2022-2023 Andrew D. Straw.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! Data representations for YCbCr image data

use super::*;

/// An image in YCbCr format
///
/// This references data stored elsewhere and provides only minimal metadata to
/// describe the actual image data. The encoder reads complete padded
/// macroblocks, so every byte in the padded rows and columns must be initialized
/// before encoding. Those padding samples are included in the encoded stream
/// even though the sequence parameter set crops the visible picture; callers
/// should fill padding (for example, by replicating edge samples) before data
/// crosses a trust boundary.
///
/// The luma stride must be evenly divisible by 16 and the luma data size must
/// have an integer multiple of 16 rows. For chroma, this number is 8.
pub struct YCbCrImage<'a> {
    /// The data planes for the image
    pub planes: Planes<'a>,
    /// The width of the image, in pixels
    pub width: u32,
    /// The height of the image, in pixels
    pub height: u32,
}

impl<'a> YCbCrImage<'a> {
    pub(crate) fn luma_bit_depth(&self) -> BitDepth {
        match &self.planes {
            Planes::Mono(y) => y.bit_depth,
            Planes::YCbCr((y, _, _)) => y.bit_depth,
        }
    }
}

/// The data plane(s) within an [YCbCrImage].
pub enum Planes<'a> {
    //// Luminance only (monochrome) data.
    Mono(DataPlane<'a>),
    //// Luminance and chrominance data.
    YCbCr((DataPlane<'a>, DataPlane<'a>, DataPlane<'a>)),
}

/// Data for a single plane (luminance or chrominance) of an image.
///
/// The actual data are stored elsewhere and this provides metadata.
pub struct DataPlane<'a> {
    /// The image data
    pub data: &'a [u8],
    /// The row stride of the image data
    pub stride: usize,
    /// The bit depth of the image data
    pub bit_depth: BitDepth,
}

impl<'a> YCbCrImage<'a> {
    pub(crate) fn check_sizes(&self) -> Result<()> {
        match &self.planes {
            Planes::Mono(y_plane) | Planes::YCbCr((y_plane, _, _)) => {
                y_plane.check_sizes(self.width, self.height, 16)?;
            }
        }

        match &self.planes {
            Planes::Mono(_) => {}
            Planes::YCbCr((_, cb_plane, cr_plane)) => {
                for chroma_plane in [cb_plane, cr_plane] {
                    chroma_plane.check_sizes(self.width / 2, self.height / 2, 8)?;
                }
            }
        }
        Ok(())
    }
}

impl<'a> DataPlane<'a> {
    pub(crate) fn check_sizes(&self, width: u32, height: u32, mb_sz: u32) -> Result<()> {
        let fail = || Error::DataShapeProblem {
            msg: "invalid or incomplete padded data plane",
            #[cfg(feature = "backtrace")]
            backtrace: Backtrace::capture(),
        };
        if width == 0 || height == 0 || self.stride == 0 || mb_sz == 0 {
            return Err(fail());
        }

        let block = u64::from(mb_sz);
        let padded_width = (u64::from(width) + block - 1) / block * block;
        let padded_height = (u64::from(height) + block - 1) / block * block;
        let row_bytes = match self.bit_depth {
            BitDepth::Depth8 => padded_width,
            BitDepth::Depth12 => padded_width * 3 / 2,
        };
        let row_bytes = usize::try_from(row_bytes).map_err(|_| fail())?;
        let rows = usize::try_from(padded_height).map_err(|_| fail())?;
        let required = self.stride.checked_mul(rows).ok_or_else(fail)?;
        if self.stride < row_bytes || self.data.len() < required {
            return Err(fail());
        }

        Ok(())
    }
}
