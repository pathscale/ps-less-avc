// Copyright 2022-2023 Andrew D. Straw.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! Associates an encoder with a writer to allow writing encoded frames.

use std::io::Write;

use super::{Error, LessEncoder, Result, YCbCrImage};

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
    /// Encoding errors leave the writer and encoding session available for a
    /// corrected retry. Once I/O fails, the sink may contain a committed prefix;
    /// later writes are rejected, while `into_inner` still returns the sink.
    pub fn write(&mut self, frame: &YCbCrImage) -> Result<()> {
        if self.io_failed {
            return Err(Error::InconsistentState {
                #[cfg(feature = "backtrace")]
                backtrace: std::backtrace::Backtrace::capture(),
            });
        }

        if self.encoder.is_none() {
            // Prepare the encoder and all three NALs before touching the sink.
            let (initial, encoder) = LessEncoder::new(frame)?;
            let sps = initial.sps.to_annex_b_data();
            let pps = initial.pps.to_annex_b_data();
            let picture = initial.frame.to_annex_b_data();

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

        // LessEncoder::encode validates and builds the NAL before any sink I/O.
        let picture = self
            .encoder
            .as_mut()
            .expect("encoder checked above")
            .encode(frame)?
            .to_annex_b_data();
        // Keep the session terminal if sink I/O unwinds after committing a prefix.
        self.io_failed = true;
        if let Err(error) = self.wtr.write_all(&picture) {
            return Err(error.into());
        }
        self.io_failed = false;
        Ok(())
    }
}
