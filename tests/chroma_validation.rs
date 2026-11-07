use less_avc::ycbcr_image::{DataPlane, Planes, YCbCrImage};
use less_avc::{BitDepth, Error, LessEncoder};

fn padded_len(width: u32, height: u32, block: u32, depth: BitDepth) -> usize {
    let w = ((width + block - 1) / block) * block;
    let h = ((height + block - 1) / block) * block;
    let row = match depth {
        BitDepth::Depth8 => w as usize,
        BitDepth::Depth12 => (w as usize) * 3 / 2,
    };
    row * h as usize
}

fn color<'a>(
    width: u32,
    height: u32,
    y_depth: BitDepth,
    cb_depth: BitDepth,
    cr_depth: BitDepth,
    y: &'a [u8],
    cb: &'a [u8],
    cr: &'a [u8],
) -> YCbCrImage<'a> {
    let plane = |data, bit_depth, width, block| DataPlane {
        data,
        stride: match bit_depth {
            BitDepth::Depth8 => ((width + block - 1) / block * block) as usize,
            BitDepth::Depth12 => ((width + block - 1) / block * block) as usize * 3 / 2,
        },
        bit_depth,
    };
    YCbCrImage {
        planes: Planes::YCbCr((
            plane(y, y_depth, width, 16),
            plane(cb, cb_depth, width / 2, 8),
            plane(cr, cr_depth, width / 2, 8),
        )),
        width,
        height,
    }
}

fn color_storage(width: u32, height: u32, depth: BitDepth) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    (
        vec![0; padded_len(width, height, 16, depth)],
        vec![0; padded_len(width / 2, height / 2, 8, depth)],
        vec![0; padded_len(width / 2, height / 2, 8, depth)],
    )
}

fn set_visible_samples(data: &mut [u8], stride: usize, width: u32, height: u32, depth: BitDepth) {
    let visible_row_bytes = match depth {
        BitDepth::Depth8 => width as usize,
        BitDepth::Depth12 => width as usize * 3 / 2,
    };
    for row in 0..height as usize {
        for (col, byte) in data[row * stride..row * stride + visible_row_bytes]
            .iter_mut()
            .enumerate()
        {
            *byte = (row.wrapping_mul(29).wrapping_add(col) % 251) as u8;
        }
    }
}

fn encoded_nals(image: &YCbCrImage<'_>) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    let (nals, mut encoder) = LessEncoder::new(image).unwrap();
    (
        nals.sps.to_annex_b_data(),
        nals.pps.to_annex_b_data(),
        nals.frame.to_annex_b_data(),
        encoder.encode(image).unwrap().to_annex_b_data(),
    )
}

#[test]
fn cropped_padding_sentinels_do_not_change_encoded_nals() {
    for (width, height, depth, mono) in [
        (17, 17, BitDepth::Depth8, true),
        (18, 18, BitDepth::Depth12, true),
        (18, 18, BitDepth::Depth8, false),
        (20, 18, BitDepth::Depth12, false),
    ] {
        let block = 16;
        let y_stride = match depth {
            BitDepth::Depth8 => (width + block - 1) / block * block,
            BitDepth::Depth12 => (width + block - 1) / block * block * 3 / 2,
        } as usize;
        let y_len = y_stride * ((height + 15) / 16 * 16) as usize;
        let mut y_a = vec![0xA5; y_len];
        let mut y_b = vec![0x5A; y_len];
        set_visible_samples(&mut y_a, y_stride, width, height, depth);
        set_visible_samples(&mut y_b, y_stride, width, height, depth);

        let result = if mono {
            let image_a = YCbCrImage {
                planes: Planes::Mono(DataPlane {
                    data: &y_a,
                    stride: y_stride,
                    bit_depth: depth,
                }),
                width,
                height,
            };
            let image_b = YCbCrImage {
                planes: Planes::Mono(DataPlane {
                    data: &y_b,
                    stride: y_stride,
                    bit_depth: depth,
                }),
                width,
                height,
            };
            (encoded_nals(&image_a), encoded_nals(&image_b))
        } else {
            let cb_width = width / 2;
            let cb_height = height / 2;
            let cb_stride = match depth {
                BitDepth::Depth8 => ((cb_width + 7) / 8 * 8) as usize,
                BitDepth::Depth12 => ((cb_width + 7) / 8 * 8) as usize * 3 / 2,
            };
            let cb_len = cb_stride * ((cb_height + 7) / 8 * 8) as usize;
            let (mut cb_a, mut cr_a, mut cb_b, mut cr_b) = (
                vec![0xA5; cb_len],
                vec![0xA5; cb_len],
                vec![0x5A; cb_len],
                vec![0x5A; cb_len],
            );
            set_visible_samples(&mut cb_a, cb_stride, cb_width, cb_height, depth);
            set_visible_samples(&mut cb_b, cb_stride, cb_width, cb_height, depth);
            set_visible_samples(&mut cr_a, cb_stride, cb_width, cb_height, depth);
            set_visible_samples(&mut cr_b, cb_stride, cb_width, cb_height, depth);
            let image_a = color(width, height, depth, depth, depth, &y_a, &cb_a, &cr_a);
            let image_b = color(width, height, depth, depth, depth, &y_b, &cb_b, &cr_b);
            (encoded_nals(&image_a), encoded_nals(&image_b))
        };
        assert_eq!(
            result.0, result.1,
            "padding sentinel changed output for {width}x{height} {depth:?}"
        );
    }
}

fn assert_short_chroma_plane(reject_cb: bool) {
    let width = 16;
    let height = 16;
    let y = vec![0; padded_len(width, height, 16, BitDepth::Depth8)];
    let cb_len = padded_len(width / 2, height / 2, 8, BitDepth::Depth8);
    let cr_len = cb_len;
    let cb = vec![0; if reject_cb { cb_len - 1 } else { cb_len }];
    let cr = vec![0; if reject_cb { cr_len } else { cr_len - 1 }];
    let image = color(
        width,
        height,
        BitDepth::Depth8,
        BitDepth::Depth8,
        BitDepth::Depth8,
        &y,
        &cb,
        &cr,
    );
    assert_shape_error(&image, "invalid or incomplete padded data plane");
}

fn assert_shape_error(image: &YCbCrImage<'_>, expected: &'static str) {
    match LessEncoder::new(image) {
        Err(Error::DataShapeProblem { msg, .. }) => assert_eq!(msg, expected),
        _ => panic!("expected data-shape error: {expected}"),
    }
}

fn assert_unsupported_size(image: &YCbCrImage<'_>) {
    assert!(matches!(
        LessEncoder::new(image),
        Err(Error::UnsupportedImageSize { .. })
    ));
}

#[test]
fn odd_color_width_is_rejected_before_storage_validation() {
    for width in [1, 17] {
        let height = 16;
        let (y, cb, cr) = color_storage(width, height, BitDepth::Depth8);
        let image = color(
            width,
            height,
            BitDepth::Depth8,
            BitDepth::Depth8,
            BitDepth::Depth8,
            &y,
            &cb,
            &cr,
        );
        assert_shape_error(&image, "width must be divisible by 2 (except mono8)");
    }
}

#[test]
fn odd_color_height_is_rejected_before_storage_validation() {
    for height in [1, 17] {
        let width = 16;
        let (y, cb, cr) = color_storage(width, height, BitDepth::Depth8);
        let image = color(
            width,
            height,
            BitDepth::Depth8,
            BitDepth::Depth8,
            BitDepth::Depth8,
            &y,
            &cb,
            &cr,
        );
        assert_unsupported_size(&image);
    }
}

#[test]
fn color_alignment_precedes_short_chroma_storage() {
    let (y, _cb, cr) = color_storage(16, 17, BitDepth::Depth8);
    let image = color(
        16,
        17,
        BitDepth::Depth8,
        BitDepth::Depth8,
        BitDepth::Depth8,
        &y,
        &[],
        &cr,
    );
    assert_unsupported_size(&image);
    let (y, _cb, cr) = color_storage(17, 16, BitDepth::Depth8);
    let image = color(
        17,
        16,
        BitDepth::Depth8,
        BitDepth::Depth8,
        BitDepth::Depth8,
        &y,
        &[],
        &cr,
    );
    assert_shape_error(&image, "width must be divisible by 2 (except mono8)");
}

#[test]
fn each_chroma_plane_depth_must_match_luma() {
    for (cb_depth, cr_depth) in [
        (BitDepth::Depth12, BitDepth::Depth8),
        (BitDepth::Depth8, BitDepth::Depth12),
    ] {
        let width = 16;
        let height = 16;
        let y = vec![0; padded_len(width, height, 16, BitDepth::Depth8)];
        let cb = vec![0; padded_len(width / 2, height / 2, 8, cb_depth)];
        let cr = vec![0; padded_len(width / 2, height / 2, 8, cr_depth)];
        let image = color(
            width,
            height,
            BitDepth::Depth8,
            cb_depth,
            cr_depth,
            &y,
            &cb,
            &cr,
        );
        assert_shape_error(&image, "YCbCr planes must have matching bit depths");
    }
}

#[test]
fn color12_width_must_be_divisible_by_four() {
    let width = 18;
    let height = 16;
    let (y, cb, cr) = color_storage(width, height, BitDepth::Depth12);
    let image = color(
        width,
        height,
        BitDepth::Depth12,
        BitDepth::Depth12,
        BitDepth::Depth12,
        &y,
        &cb,
        &cr,
    );
    assert_shape_error(
        &image,
        "for bit depth 12 color, width must be divisible by 4",
    );
}

#[test]
fn padded_color8_and_color12_frames_encode_deterministically() {
    for (width, height, depth) in [(18, 18, BitDepth::Depth8), (20, 18, BitDepth::Depth12)] {
        let (y, cb, cr) = color_storage(width, height, depth);
        let image = color(width, height, depth, depth, depth, &y, &cb, &cr);
        let (nals_a, _) = LessEncoder::new(&image).unwrap();
        let (nals_b, _) = LessEncoder::new(&image).unwrap();
        assert_eq!(nals_a.sps.to_annex_b_data(), nals_b.sps.to_annex_b_data());
        assert_eq!(nals_a.pps.to_annex_b_data(), nals_b.pps.to_annex_b_data());
        assert_eq!(
            nals_a.frame.to_annex_b_data(),
            nals_b.frame.to_annex_b_data()
        );
    }
}

#[test]
fn final_padded_chroma_row_must_be_complete_in_both_planes() {
    assert_short_chroma_plane(true);
    assert_short_chroma_plane(false);
}

#[test]
fn zero_dimensions_are_rejected() {
    for (width, height) in [(0, 16), (16, 0), (0, 0)] {
        let (y, cb, cr) = color_storage(width, height, BitDepth::Depth8);
        let image = color(
            width,
            height,
            BitDepth::Depth8,
            BitDepth::Depth8,
            BitDepth::Depth8,
            &y,
            &cb,
            &cr,
        );
        assert_shape_error(&image, "invalid or incomplete padded data plane");
    }
}

#[test]
fn odd_mono8_dimensions_remain_supported() {
    let data = vec![0; 32 * 32];
    let image = YCbCrImage {
        planes: Planes::Mono(DataPlane {
            data: &data,
            stride: 32,
            bit_depth: BitDepth::Depth8,
        }),
        width: 17,
        height: 17,
    };
    assert!(LessEncoder::new(&image).is_ok());
}

#[test]
fn odd_mono12_width_is_rejected() {
    let data = vec![0; 48 * 16];
    let image = YCbCrImage {
        planes: Planes::Mono(DataPlane {
            data: &data,
            stride: 48,
            bit_depth: BitDepth::Depth12,
        }),
        width: 17,
        height: 16,
    };
    assert_shape_error(&image, "width must be divisible by 2 (except mono8)");
}

#[test]
fn rejected_mixed_depth_frame_does_not_change_the_session() {
    let width = 16;
    let height = 16;
    let (y, cb, cr) = color_storage(width, height, BitDepth::Depth8);
    let valid = color(
        width,
        height,
        BitDepth::Depth8,
        BitDepth::Depth8,
        BitDepth::Depth8,
        &y,
        &cb,
        &cr,
    );
    let mut recovered = LessEncoder::new(&valid).unwrap().1;
    let mut clean = LessEncoder::new(&valid).unwrap().1;

    let y_bad = vec![0; padded_len(width, height, 16, BitDepth::Depth8)];
    let cb_bad = vec![0; padded_len(width / 2, height / 2, 8, BitDepth::Depth12)];
    let cr_bad = vec![0; padded_len(width / 2, height / 2, 8, BitDepth::Depth8)];
    let mixed = color(
        width,
        height,
        BitDepth::Depth8,
        BitDepth::Depth12,
        BitDepth::Depth8,
        &y_bad,
        &cb_bad,
        &cr_bad,
    );
    assert!(matches!(
        recovered.encode(&mixed),
        Err(Error::DataShapeProblem {
            msg: "frame geometry or format changed; recreate the encoder",
            ..
        })
    ));

    let next_recovered = recovered.encode(&valid).unwrap().to_annex_b_data();
    let next_clean = clean.encode(&valid).unwrap().to_annex_b_data();
    assert_eq!(next_recovered, next_clean);
}
