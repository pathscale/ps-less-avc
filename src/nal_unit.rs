// Copyright 2022-2023 Andrew D. Straw.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! Network Abstraction Layer (NAL) encoding

use super::*;

/// Data to save a NAL unit
///
/// The data is in the raw byte sequence payload (RBSP) representation and gets
/// converted to a NAL unit by the [Self::to_annex_b_data] method.
pub struct NalUnit {
    ref_idc: NalRefIdc,
    unit_type: NalUnitType,
    rbsp_data: RbspData,
}

impl NalUnit {
    /// Create new [NalUnit].
    pub fn new(ref_idc: NalRefIdc, unit_type: NalUnitType, rbsp_data: RbspData) -> Self {
        Self {
            ref_idc,
            unit_type,
            rbsp_data,
        }
    }

    fn try_to_buf(&self, with_start_code: bool) -> Result<Vec<u8>> {
        #[allow(clippy::identity_op)]
        // forbidden_zero_bit = 0
        let nal_byte = 0x00 | (self.ref_idc.nal_ref_idc() << 5) | self.unit_type.nal_unit_type();

        let rbsp_buf = &self.rbsp_data.data;
        let header_size = if with_start_code { 5 } else { 1 };
        let result_size = calc_serialized_buf_size(rbsp_buf.len(), header_size)?;
        let mut result = Vec::new();
        result
            .try_reserve_exact(result_size)
            .map_err(|_| capacity_error())?;
        result.resize(result_size, 0);
        if with_start_code {
            result[..4].copy_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        }
        result[header_size - 1] = nal_byte;

        let nal_buf_sz = rbsp_to_ebsp(rbsp_buf, &mut result[header_size..])?;
        result.truncate(header_size + nal_buf_sz);

        Ok(result)
    }

    /// Return the NAL header followed by the EBSP.
    ///
    /// Panics if the output buffer cannot be allocated.
    pub fn to_nal_unit(&self) -> Vec<u8> {
        self.try_to_nal_unit().expect("NAL unit serialization failed")
    }

    /// Return the NAL header followed by the EBSP.
    ///
    /// Returns [Error::UnsupportedImageSize] if output sizing or reservation fails.
    pub fn try_to_nal_unit(&self) -> Result<Vec<u8>> {
        self.try_to_buf(false)
    }

    /// Return a single NAL unit encoded for direct saving to `.h264` file.
    ///
    /// Panics if the output buffer cannot be allocated.
    pub fn to_annex_b_data(&self) -> Vec<u8> {
        self.try_to_annex_b_data()
            .expect("Annex B serialization failed")
    }

    /// Return a four-byte Annex B start code followed by the NAL header and EBSP.
    ///
    /// Returns [Error::UnsupportedImageSize] if output sizing or reservation fails.
    pub fn try_to_annex_b_data(&self) -> Result<Vec<u8>> {
        self.try_to_buf(true)
    }
}

fn capacity_error() -> Error {
    // Capturing a backtrace could allocate after a failed reservation.
    Error::UnsupportedImageSize {
        #[cfg(feature = "backtrace")]
        backtrace: Backtrace::disabled(),
    }
}

/// Calculate the maximum serialized buffer size for a given RBSP size.
#[inline]
fn calc_serialized_buf_size(rbsp_size: usize, header_size: usize) -> Result<usize> {
    rbsp_size
        .checked_add(rbsp_size / 2)
        .and_then(|payload_size| header_size.checked_add(payload_size))
        .ok_or_else(capacity_error)
}

/// Convert Raw byte sequence payload (RBSP) data to Encapsulated Byte Sequence
/// Payload (EBSP) bytes.
pub(crate) fn rbsp_to_ebsp(rbsp_buf: &[u8], nal_buf: &mut [u8]) -> Result<usize> {
    if nal_buf.len() < calc_serialized_buf_size(rbsp_buf.len(), 0)? {
        return Err(capacity_error());
    }
    let mut dest_len = 0;

    let mut input_buf = rbsp_buf;

    while let Some(first_idx) = input_buf.iter().position(|&b| b == 0x00) {
        if first_idx + 1 < input_buf.len() {
            // more input exists
            if input_buf[first_idx + 1] == 0x00 {
                // two nulls in a row
                if first_idx + 2 < input_buf.len() {
                    // it is longer
                    let pos3 = input_buf[first_idx + 2];
                    if needs_protecting_in_pos3(pos3) {
                        let src = &input_buf[..first_idx + 2];
                        nal_buf[dest_len..dest_len + src.len()].copy_from_slice(src);
                        dest_len += src.len();
                        nal_buf[dest_len] = 0x03;
                        dest_len += 1;
                        input_buf = &input_buf[src.len()..];
                    } else {
                        let src = &input_buf[..first_idx + 2];
                        nal_buf[dest_len..dest_len + src.len()].copy_from_slice(src);
                        dest_len += src.len();
                        input_buf = &input_buf[src.len()..];
                    }
                } else {
                    // no more input
                    break;
                }
            } else {
                // next index is not null, use input up to and including null
                let src = &input_buf[..first_idx + 1];
                nal_buf[dest_len..dest_len + src.len()].copy_from_slice(src);
                dest_len += src.len();
                input_buf = &input_buf[src.len()..];
            }
        } else {
            // no more input
            break;
        }
    }

    if !input_buf.is_empty() {
        nal_buf[dest_len..dest_len + input_buf.len()].copy_from_slice(input_buf);
        dest_len += input_buf.len();
    }

    Ok(dest_len)
}

#[inline]
/// Returns true if byte is 0x00, 0x01, 0x02 or 0x03.
fn needs_protecting_in_pos3(byte: u8) -> bool {
    matches!(byte, 0x00 | 0x01 | 0x02 | 0x03)
}

#[test]
fn test_bad_byte() {
    assert!(needs_protecting_in_pos3(0x00));
    assert!(needs_protecting_in_pos3(0x01));
    assert!(needs_protecting_in_pos3(0x02));
    assert!(needs_protecting_in_pos3(0x03));
    assert!(!needs_protecting_in_pos3(0x04));
    for byte in 4..=255 {
        assert!(!needs_protecting_in_pos3(byte));
    }
}

#[test]
fn test_nal_encoding_roundtrip() {
    // `h264_reader::rbsp::decode_nal` trims the first byte.
    let test_vecs = [
        vec![0x68, 0x00],
        vec![0x68, 0x01],
        vec![0x68, 0x02],
        vec![0x68, 0x03],
        vec![0x68, 0x04],
        vec![0x68, 0x00, 0x00],
        vec![0x68, 0x00, 0x01],
        vec![0x68, 0x00, 0x02],
        vec![0x68, 0x00, 0x03],
        vec![0x68, 0x00, 0x04],
        vec![0x68, 0x00, 0x00, 0x00],
        vec![0x68, 0x00, 0x00, 0x01],
        vec![0x68, 0x00, 0x00, 0x02],
        vec![0x68, 0x00, 0x00, 0x03],
        vec![0x68, 0x00, 0x00, 0x04],
        vec![0x68, 0x00, 0x00, 0x00, 0x00],
        vec![0x68, 0x00, 0x00, 0x00, 0x01],
        vec![0x68, 0x00, 0x00, 0x00, 0x02],
        vec![0x68, 0x00, 0x00, 0x00, 0x03],
        vec![0x68, 0x00, 0x00, 0x00, 0x04],
        vec![0x68, 0x00, 0x00, 0x00, 0x05],
        vec![0x68, 0x03, 0x03, 0x03, 0x03],
        vec![0x68, 0x00, 0x00, 0x00, 0x00, 0x00],
        vec![0x68, 0x00, 0x00, 0x00, 0x01, 0x00],
        vec![0x68, 0x00, 0x00, 0x00, 0x02, 0x00],
        vec![0x68, 0x00, 0x00, 0x00, 0x03, 0x00],
        vec![0x68, 0x00, 0x00, 0x00, 0x04, 0x00],
        vec![0x68, 0x00, 0x00, 0x00, 0x05, 0x00],
        vec![0x68, 0x03, 0x03, 0x03, 0x03, 0x03],
        vec![0x68, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00],
        vec![0x68, 0x00, 0x00, 0x03, 0x00, 0x00, 0x01],
        vec![0x68, 0x00, 0x00, 0x03, 0x00, 0x00, 0x02],
        vec![0x68, 0x00, 0x00, 0x03, 0x00, 0x00, 0x03],
        vec![0x68, 0x00, 0x00, 0x03, 0x00, 0x00, 0x04],
        vec![0x68, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        vec![0x68, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01],
        vec![0x68, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        vec![0x68, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01],
        vec![0x68, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02],
        vec![0x68, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03],
        vec![0x68, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04],
    ];
    for orig in test_vecs.iter() {
        let mut encoded = vec![0u8; calc_serialized_buf_size(orig.len(), 0).unwrap()];
        let sz = rbsp_to_ebsp(orig, &mut encoded).unwrap();
        encoded.truncate(sz);

        let decoded = h264_reader::rbsp::decode_nal(&encoded).unwrap();
        assert_eq!(&orig.as_slice()[1..], decoded.as_ref());
    }
}

#[test]
fn serialized_size_overflow_is_reported() {
    for (rbsp_size, header_size) in [(usize::MAX, 0), (1, usize::MAX)] {
        assert!(matches!(
            calc_serialized_buf_size(rbsp_size, header_size),
            Err(Error::UnsupportedImageSize { .. })
        ));
    }
}

#[test]
fn rbsp_conversion_rejects_a_short_buffer_without_writing() {
    let mut output = [0xaa; 3];
    assert!(matches!(
        rbsp_to_ebsp(&[0x00, 0x00, 0x01], &mut output),
        Err(Error::UnsupportedImageSize { .. })
    ));
    assert_eq!(output, [0xaa; 3]);
}

#[test]
fn serializers_emit_expected_header_and_escaped_payload() {
    let nal = NalUnit::new(
        NalRefIdc::One,
        NalUnitType::CodedSliceOfAnIDRPicture,
        RbspData {
            data: vec![0x00, 0x00, 0x01, 0x00, 0x00, 0x03, 0x80],
        },
    );

    let expected_nal = [0x25, 0x00, 0x00, 0x03, 0x01, 0x00, 0x00, 0x03, 0x03, 0x80];
    assert_eq!(nal.try_to_nal_unit().unwrap().as_slice(), &expected_nal);
    assert_eq!(nal.to_nal_unit().as_slice(), &expected_nal);

    let annex_b = nal.try_to_annex_b_data().unwrap();
    assert_eq!(&annex_b[..4], &[0x00, 0x00, 0x00, 0x01]);
    assert_eq!(&annex_b[4..], &expected_nal);
}

/// Possible values for the `nal_ref_idc` field in the `nal_unit`.
///
/// Encodes to 2 bits.
pub enum NalRefIdc {
    // TODO: could these have better names?
    Zero,
    One,
    Two,
    Three,
}

impl NalRefIdc {
    pub(crate) fn nal_ref_idc(&self) -> u8 {
        match self {
            Self::Zero => 0,
            Self::One => 1,
            Self::Two => 2,
            Self::Three => 3,
        }
    }
}

/// Possible values for the `nal_unit_type` field in `nal_unit`.
///
/// Encodes to 5 bits.
#[allow(dead_code)]
#[derive(PartialEq, Eq)]
#[non_exhaustive]
pub enum NalUnitType {
    /// Unspecified
    Unspecified,
    /// Coded slice of a non-IDR picture
    CodedSliceOfANonIDRPicture,
    /// Coded slice data partition A
    CodedSliceDataPartitionA,
    /// Coded slice data partition B
    CodedSliceDataPartitionB,
    /// Coded slice data partition C
    CodedSliceDataPartitionC,
    /// Coded slice of an IDR picture
    CodedSliceOfAnIDRPicture,
    /// Supplemental enhancement information (SEI)
    SupplementalEnhancementInformation,
    /// Sequence parameter set
    SequenceParameterSet,
    /// Picture parameter set
    PictureParameterSet,
    // There are more, which is why this is marked `non_exhaustive`.
}

impl NalUnitType {
    pub(crate) fn nal_unit_type(&self) -> u8 {
        match self {
            Self::Unspecified => 0,
            Self::CodedSliceOfANonIDRPicture => 1,
            Self::CodedSliceDataPartitionA => 2,
            Self::CodedSliceDataPartitionB => 3,
            Self::CodedSliceDataPartitionC => 4,
            Self::CodedSliceOfAnIDRPicture => 5,
            Self::SupplementalEnhancementInformation => 6,
            Self::SequenceParameterSet => 7,
            Self::PictureParameterSet => 8,
        }
    }
}

/// The initial [NalUnit] returned when starting a [LessEncoder].
pub struct InitialNalUnits {
    /// sequence parameter set NAL unit
    pub sps: NalUnit,
    /// picture parameter set NAL unit
    pub pps: NalUnit,
    /// frame NAL unit
    pub frame: NalUnit,
}

impl InitialNalUnits {
    /// Return an [Iterator] over the NAL units generated at the start of encoding.
    pub fn into_iter(self) -> impl Iterator<Item = NalUnit> {
        vec![self.sps, self.pps, self.frame].into_iter()
    }
}
