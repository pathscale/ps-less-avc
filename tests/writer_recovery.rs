#![cfg(feature = "std")]

use std::cell::Cell;
use std::io::{self, Write};
use std::rc::Rc;

use less_avc::ycbcr_image::{DataPlane, Planes, YCbCrImage};
use less_avc::{BitDepth, Error, H264Writer, LessEncoder};

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

fn expected_output(first: &[u8], second: Option<&[u8]>) -> Vec<u8> {
    let (initial, mut encoder) = LessEncoder::new(&mono8(first)).unwrap();
    let mut output = Vec::new();
    output.extend_from_slice(&initial.sps.to_annex_b_data());
    output.extend_from_slice(&initial.pps.to_annex_b_data());
    output.extend_from_slice(&initial.frame.to_annex_b_data());
    if let Some(second) = second {
        let picture = encoder.encode(&mono8(second)).unwrap();
        output.extend_from_slice(&picture.to_annex_b_data());
    }
    output
}

fn initial_nal_lengths(frame: &[u8]) -> [usize; 3] {
    let (initial, _) = LessEncoder::new(&mono8(frame)).unwrap();
    [
        initial.sps.to_annex_b_data().len(),
        initial.pps.to_annex_b_data().len(),
        initial.frame.to_annex_b_data().len(),
    ]
}

#[test]
fn encoding_errors_leave_sink_and_session_recoverable() {
    let valid = vec![0x31; 16 * 16];
    let invalid = [0x00; 1];

    let mut initial = H264Writer::new(Vec::new()).unwrap();
    assert!(matches!(
        initial.write(&mono8(&invalid)),
        Err(Error::DataShapeProblem { .. })
    ));
    assert!(initial.into_inner().is_empty());

    let mut recovered_initial = H264Writer::new(Vec::new()).unwrap();
    assert!(matches!(
        recovered_initial.write(&mono8(&invalid)),
        Err(Error::DataShapeProblem { .. })
    ));
    recovered_initial.write(&mono8(&valid)).unwrap();
    assert_eq!(recovered_initial.into_inner(), expected_output(&valid, None));

    let second = vec![0x72; 16 * 16];
    let mut recording = H264Writer::new(Vec::new()).unwrap();
    recording.write(&mono8(&valid)).unwrap();
    assert!(recording.write(&mono8(&invalid)).is_err());
    recording.write(&mono8(&second)).unwrap();
    assert_eq!(recording.into_inner(), expected_output(&valid, Some(&second)));
}

struct PrefixThenFail {
    bytes: Vec<u8>,
    limit: usize,
    calls: Rc<Cell<usize>>,
}

impl Write for PrefixThenFail {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.calls.set(self.calls.get() + 1);
        let remaining = self.limit.saturating_sub(self.bytes.len());
        if remaining == 0 {
            return Err(io::Error::new(io::ErrorKind::Other, "injected sink failure"));
        }
        let accepted = remaining.min(buf.len());
        self.bytes.extend_from_slice(&buf[..accepted]);
        Ok(accepted)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn assert_terminal_io_failure(
    limit: usize,
    expected: &[u8],
    valid: &[u8],
    second: &[u8],
    initial_len: usize,
) {
    let sink = PrefixThenFail {
        bytes: Vec::new(),
        limit,
        calls: Rc::new(Cell::new(0)),
    };
    let call_counter = Rc::clone(&sink.calls);
    let mut writer = H264Writer::new(sink).unwrap();
    let error = if limit > initial_len {
        writer.write(&mono8(valid)).unwrap();
        writer.write(&mono8(second)).unwrap_err()
    } else {
        writer.write(&mono8(valid)).unwrap_err()
    };
    match error {
        Error::IoError { source, .. } => assert_eq!(source.to_string(), "injected sink failure"),
        other => panic!("expected the original I/O error, got {other:?}"),
    }

    let calls_after_failure = call_counter.get();
    assert!(matches!(
        writer.write(&mono8(valid)),
        Err(Error::InconsistentState { .. })
    ));
    assert_eq!(call_counter.get(), calls_after_failure);
    let sink = writer.into_inner();
    assert_eq!(sink.bytes, expected[..limit]);
}

struct PrefixThenPanic {
    bytes: Vec<u8>,
    panic_on_call: usize,
    calls: Rc<Cell<usize>>
}

impl Write for PrefixThenPanic {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let call = self.calls.get() + 1;
        self.calls.set(call);
        if call == self.panic_on_call {
            self.bytes.push(buf[0]);
            panic!("injected sink panic");
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn assert_terminal_panic(
    panic_on_call: usize,
    expected: &[u8],
    valid: &[u8],
    second: &[u8],
) {
    let sink = PrefixThenPanic {
        bytes: Vec::new(),
        panic_on_call,
        calls: Rc::new(Cell::new(0)),
    };
    let call_counter = Rc::clone(&sink.calls);
    let mut writer = H264Writer::new(sink).unwrap();
    let mut completed_initial = Vec::new();
    if panic_on_call > 1 {
        writer.write(&mono8(valid)).unwrap();
        completed_initial = expected_output(valid, None);
    }
    let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if panic_on_call == 1 {
            let _ = writer.write(&mono8(valid));
        } else {
            let _ = writer.write(&mono8(second));
        }
    }));
    assert!(panic_result.is_err());
    let calls_after_panic = call_counter.get();
    assert!(matches!(
        writer.write(&mono8(valid)),
        Err(Error::InconsistentState { .. })
    ));
    assert_eq!(call_counter.get(), calls_after_panic);
    let sink = writer.into_inner();
    let mut expected_prefix = completed_initial;
    if panic_on_call == 1 {
        expected_prefix.push(expected[0]);
    } else {
        expected_prefix.push(expected_output(valid, Some(second))[expected_output(valid, None).len()]);
    }
    assert_eq!(sink.bytes, expected_prefix);
}

#[test]
fn caught_sink_panics_are_terminal_in_both_write_phases() {
    let first = vec![0x31; 16 * 16];
    let second = vec![0x72; 16 * 16];
    let initial = expected_output(&first, None);
    let full = expected_output(&first, Some(&second));
    assert_terminal_panic(1, &initial, &first, &second);
    assert_terminal_panic(4, &full, &first, &second);
}

#[test]
fn io_failures_are_terminal_and_preserve_the_committed_prefix() {
    let first = vec![0x31; 16 * 16];
    let second = vec![0x72; 16 * 16];
    let initial = expected_output(&first, None);
    let full = expected_output(&first, Some(&second));
    let lengths = initial_nal_lengths(&first);
    let sps_end = lengths[0];
    let pps_end = sps_end + lengths[1];
    let initial_end = pps_end + lengths[2];

    // Fail while writing each initial NAL, then while writing a recording NAL.
    for limit in [1, sps_end + 1, pps_end + 1, initial_end + 1] {
        assert_terminal_io_failure(limit, &full, &first, &second, initial_end);
    }
}
