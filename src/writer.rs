// Copyright 2022-2023 Andrew D. Straw.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! Associates an encoder with a writer to allow writing encoded frames.

use std::io::Write;

use super::{nal_unit::NalUnit, Error, LessEncoder, Result, YCbCrImage};

/// Write images to an [std::io::Write] implementation in `.h264` file format.
pub struct H264Writer<W> {
    wtr: W,
    encoder: Option<LessEncoder>,
    io_failed: bool,
}

impl<W: Write> H264Writer<W> {
    /// Create a new [H264Writer] from an [std::io::Write] implementation.
    pub fn new(wtr: W) -> Result<Self> {
        Ok(Self {
            wtr,
            encoder: None,
            io_failed: false,
        })
    }

    /// Retrieve the underlying [std::io::Write] implementation.
    pub fn into_inner(self) -> W {
        self.wtr
    }

    /// Encode and write a frame.
    ///
    /// Encoding and serialization errors leave the writer and encoding session
    /// available for a retry. Once I/O fails, the sink may contain a committed
    /// prefix. Later writes are rejected, while `into_inner` still returns the sink.
    pub fn write(&mut self, frame: &YCbCrImage) -> Result<()> {
        self.write_with_serializer(frame, NalUnit::try_to_annex_b_data)
    }

    fn write_with_serializer(
        &mut self,
        frame: &YCbCrImage,
        mut serialize: impl FnMut(&NalUnit) -> Result<Vec<u8>>,
    ) -> Result<()> {
        if self.io_failed {
            return Err(Error::InconsistentState {
                #[cfg(feature = "backtrace")]
                backtrace: std::backtrace::Backtrace::capture(),
            });
        }

        if self.encoder.is_none() {
            // Prepare the encoder and all three NALs before touching the sink.
            let (initial, encoder) = LessEncoder::new(frame)?;
            let sps = serialize(&initial.sps)?;
            let pps = serialize(&initial.pps)?;
            let picture = serialize(&initial.frame)?;

            // Once sink I/O starts, a panic may still leave a committed prefix.
            self.io_failed = true;
            let write_result: Result<()> = (|| {
                self.wtr.write_all(&sps)?;
                self.wtr.write_all(&pps)?;
                self.wtr.write_all(&picture)?;
                Ok(())
            })();
            if let Err(error) = write_result {
                return Err(error);
            }
            self.io_failed = false;
            self.encoder = Some(encoder);
            return Ok(());
        }

        let nal = self
            .encoder
            .as_ref()
            .expect("encoder checked above")
            .encode_frame(frame)?;
        let picture = serialize(&nal)?;
        drop(nal);
        // Keep the session terminal if sink I/O unwinds after committing a prefix.
        self.io_failed = true;
        if let Err(error) = self.wtr.write_all(&picture) {
            return Err(error.into());
        }
        self.io_failed = false;
        Ok(())
    }
}

#[test]
fn serialization_errors_leave_sink_and_session_recoverable() {
    use crate::{
        ycbcr_image::{DataPlane, Planes},
        BitDepth,
    };

    fn mono8(data: &[u8]) -> YCbCrImage<'_> {
        YCbCrImage {
            planes: Planes::Mono(DataPlane {
                data,
                stride: 16,
                bit_depth: BitDepth::Depth8,
            }),
            width: 16,
            height: 16,
        }
    }

    let capacity_failure = || Error::UnsupportedImageSize {
        #[cfg(feature = "backtrace")]
        backtrace: std::backtrace::Backtrace::disabled(),
    };
    let first_data = [0x31; 256];
    let next_data = [0x72; 256];
    let first = mono8(&first_data);
    let next = mono8(&next_data);
    let mut writer = H264Writer::new(Vec::new()).unwrap();
    let mut clean = H264Writer::new(Vec::new()).unwrap();

    let mut calls = 0;
    let failure = writer.write_with_serializer(&first, |nal| {
        calls += 1;
        if calls == 3 {
            Err(capacity_failure())
        } else {
            nal.try_to_annex_b_data()
        }
    });
    assert!(matches!(failure, Err(Error::UnsupportedImageSize { .. })));
    assert!(writer.wtr.is_empty());

    writer.write(&first).unwrap();
    clean.write(&first).unwrap();
    assert_eq!(writer.wtr, clean.wtr);

    let failure = writer.write_with_serializer(&next, |_| Err(capacity_failure()));
    assert!(matches!(failure, Err(Error::UnsupportedImageSize { .. })));
    assert_eq!(writer.wtr, clean.wtr);

    writer.write(&next).unwrap();
    clean.write(&next).unwrap();
    assert_eq!(writer.into_inner(), clean.into_inner());
}
