//! Reading raw little-endian `f32` PCM from a byte stream.

use std::io::{self, Read};

use transcribe_core::Pcm16kMono;

#[derive(Debug, thiserror::Error)]
pub enum PcmError {
    #[error("failed to read PCM from stdin")]
    Io(#[from] io::Error),
    #[error("PCM input is {total_bytes} bytes, which is not a whole number of 4-byte f32 samples")]
    TrailingBytes { total_bytes: u64 },
    #[error("PCM sample {index} is not a finite number")]
    NonFinite { index: usize },
}

const CHUNK: usize = 64 * 1024;

/// Reads little-endian `f32` samples until EOF.
///
/// Reads may end anywhere, including in the middle of a sample; partial
/// samples are carried over to the next read. A total byte count that is not
/// a multiple of 4, or a NaN/infinite sample, is an error.
pub fn read_pcm_f32le(mut reader: impl Read) -> Result<Pcm16kMono, PcmError> {
    let mut samples = Vec::new();
    let mut buf = vec![0u8; CHUNK];
    let mut carry = [0u8; 4];
    let mut carry_len = 0usize;
    let mut total_bytes: u64 = 0;

    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        total_bytes += n as u64;
        let mut data = &buf[..n];

        if carry_len > 0 {
            let take = (4 - carry_len).min(data.len());
            carry[carry_len..carry_len + take].copy_from_slice(&data[..take]);
            carry_len += take;
            data = &data[take..];
            if carry_len < 4 {
                continue;
            }
            samples.push(f32::from_le_bytes(carry));
        }

        // Whole samples in this read; the 0–3 leftover bytes carry over.
        let (whole, rest) = data.as_chunks::<4>();
        samples.extend(whole.iter().map(|c| f32::from_le_bytes(*c)));
        carry[..rest.len()].copy_from_slice(rest);
        carry_len = rest.len();
    }

    if carry_len != 0 {
        return Err(PcmError::TrailingBytes { total_bytes });
    }
    if let Some(index) = samples.iter().position(|s| !s.is_finite()) {
        return Err(PcmError::NonFinite { index });
    }
    Ok(Pcm16kMono(samples))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(samples: &[f32]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    /// Yields the input in reads of the given sizes, cycling.
    struct Chunked {
        data: Vec<u8>,
        pos: usize,
        sizes: Vec<usize>,
        call: usize,
    }

    impl Read for Chunked {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let size = self.sizes[self.call % self.sizes.len()];
            self.call += 1;
            let n = size.min(buf.len()).min(self.data.len() - self.pos);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    #[test]
    fn reads_samples() {
        let input = [0.0, 1.0, -0.5, 0.25];
        let pcm = read_pcm_f32le(&bytes(&input)[..]).unwrap();
        assert_eq!(pcm.0, input);
    }

    #[test]
    fn empty_input_is_empty_pcm() {
        assert!(read_pcm_f32le(io::empty()).unwrap().0.is_empty());
    }

    #[test]
    fn samples_split_across_reads() {
        let input: Vec<f32> = (0..100).map(|i| i as f32 / 100.0).collect();
        for sizes in [vec![1], vec![3], vec![5, 2, 7], vec![1, 1, 1, 6]] {
            let reader = Chunked {
                data: bytes(&input),
                pos: 0,
                sizes: sizes.clone(),
                call: 0,
            };
            assert_eq!(read_pcm_f32le(reader).unwrap().0, input, "sizes {sizes:?}");
        }
    }

    #[test]
    fn input_larger_than_one_chunk() {
        let input: Vec<f32> = (0..(CHUNK / 4 * 3 + 7)).map(|i| i as f32).collect();
        assert_eq!(read_pcm_f32le(&bytes(&input)[..]).unwrap().0, input);
    }

    #[test]
    fn trailing_bytes_are_an_error() {
        let mut data = bytes(&[1.0, 2.0]);
        data.extend_from_slice(&[0, 0, 0]);
        let err = read_pcm_f32le(&data[..]).unwrap_err();
        assert!(
            matches!(err, PcmError::TrailingBytes { total_bytes: 11 }),
            "{err}"
        );
    }

    #[test]
    fn non_finite_samples_are_an_error() {
        let err = read_pcm_f32le(&bytes(&[0.0, f32::NAN])[..]).unwrap_err();
        assert!(matches!(err, PcmError::NonFinite { index: 1 }), "{err}");
    }

    #[test]
    fn interrupted_reads_are_retried() {
        struct InterruptOnce(bool, &'static [u8]);
        impl Read for InterruptOnce {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if !self.0 {
                    self.0 = true;
                    return Err(io::ErrorKind::Interrupted.into());
                }
                let n = self.1.len().min(buf.len());
                buf[..n].copy_from_slice(&self.1[..n]);
                self.1 = &self.1[n..];
                Ok(n)
            }
        }
        let pcm = read_pcm_f32le(InterruptOnce(false, &[0, 0, 128, 63])).unwrap();
        assert_eq!(pcm.0, [1.0]);
    }

    #[test]
    fn read_errors_are_reported() {
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("boom"))
            }
        }
        assert!(matches!(read_pcm_f32le(Failing), Err(PcmError::Io(_))));
    }

    // --- Adversarial cases added by the test pass. -------------------------
    //
    // Expected sample values are given as hand-written IEEE 754 byte patterns
    // where endianness matters, not produced by `to_le_bytes`.

    fn chunked(data: Vec<u8>, sizes: Vec<usize>) -> Chunked {
        Chunked {
            data,
            pos: 0,
            sizes,
            call: 0,
        }
    }

    fn bits(samples: &[f32]) -> Vec<u32> {
        samples.iter().map(|s| s.to_bits()).collect()
    }

    #[test]
    fn known_little_endian_byte_patterns() {
        let data = [
            0x00, 0x00, 0x80, 0x3F, // 1.0
            0x00, 0x00, 0x80, 0xBF, // -1.0
            0x00, 0x00, 0x00, 0x3F, // 0.5
            0x00, 0x00, 0x00, 0x00, // 0.0
            0x00, 0x00, 0x00, 0x80, // -0.0
            0x01, 0x00, 0x80, 0x3F, // 1.0 + 1 ulp
            0xDB, 0x0F, 0x49, 0x40, // pi
        ];
        let pcm = read_pcm_f32le(&data[..]).unwrap();
        assert_eq!(
            bits(&pcm.0),
            vec![
                0x3F80_0000,
                0xBF80_0000,
                0x3F00_0000,
                0x0000_0000,
                0x8000_0000,
                0x3F80_0001,
                0x4049_0FDB
            ]
        );
    }

    #[test]
    fn big_endian_interpretation_is_not_used() {
        // 0x3F800000 in big-endian order would be 1.0; read as little-endian
        // these bytes are a tiny denormal-ish value, not 1.0.
        let pcm = read_pcm_f32le(&[0x3F, 0x80, 0x00, 0x00][..]).unwrap();
        assert_eq!(pcm.0[0].to_bits(), 0x0000_803F);
        assert_ne!(pcm.0[0], 1.0);
    }

    #[test]
    fn subnormals_negative_zero_and_extremes_are_kept_bit_exact() {
        let input = [
            f32::from_bits(1), // smallest subnormal
            -0.0,
            f32::MAX,
            f32::MIN,
            f32::MIN_POSITIVE,
            -f32::EPSILON,
        ];
        let pcm = read_pcm_f32le(&bytes(&input)[..]).unwrap();
        assert_eq!(bits(&pcm.0), bits(&input));
    }

    #[test]
    fn samples_outside_unit_range_are_accepted() {
        // Nominal range is [-1, 1] but clipping is not the reader's job.
        let pcm = read_pcm_f32le(&bytes(&[2.5, -3.0, 1000.0])[..]).unwrap();
        assert_eq!(pcm.0, [2.5, -3.0, 1000.0]);
    }

    #[test]
    fn odd_byte_counts_are_errors_with_total() {
        for len in [1usize, 2, 3, 5, 6, 7, 9, 4097] {
            let err = read_pcm_f32le(&vec![0u8; len][..]).unwrap_err();
            match err {
                PcmError::TrailingBytes { total_bytes } => {
                    assert_eq!(total_bytes, len as u64, "len {len}")
                }
                other => panic!("len {len}: expected TrailingBytes, got {other}"),
            }
        }
    }

    #[test]
    fn odd_byte_counts_are_errors_when_split_across_reads() {
        for len in [1usize, 2, 3, 5, 13, 401] {
            for sizes in [vec![1], vec![2], vec![3], vec![1, 2, 3]] {
                let err = read_pcm_f32le(chunked(vec![0u8; len], sizes.clone())).unwrap_err();
                assert!(
                    matches!(err, PcmError::TrailingBytes { total_bytes } if total_bytes == len as u64),
                    "len {len} sizes {sizes:?}: {err}"
                );
            }
        }
    }

    #[test]
    fn multiple_of_four_bytes_split_into_odd_reads_is_fine() {
        let input = [1.0f32, -1.0, 0.5];
        for sizes in [
            vec![1],
            vec![2],
            vec![3],
            vec![3, 1, 2],
            vec![2, 3, 1],
            vec![5],
        ] {
            let pcm = read_pcm_f32le(chunked(bytes(&input), sizes.clone())).unwrap();
            assert_eq!(pcm.0, input, "sizes {sizes:?}");
        }
    }

    #[test]
    fn trailing_bytes_error_message_names_the_count() {
        let err = read_pcm_f32le(&[0u8; 5][..]).unwrap_err();
        assert!(err.to_string().contains('5'), "{err}");
    }

    #[test]
    fn every_kind_of_non_finite_sample_is_rejected() {
        let nans = [
            f32::NAN,
            -f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::from_bits(0x7F80_0001), // signalling NaN
            f32::from_bits(0xFFFF_FFFF), // NaN, all bits set
            f32::from_bits(0x7FC0_1234), // quiet NaN with payload
        ];
        for bad in nans {
            let err = read_pcm_f32le(&bytes(&[0.0, 0.5, bad, 0.25])[..]).unwrap_err();
            assert!(
                matches!(err, PcmError::NonFinite { index: 2 }),
                "{:#010x}: {err}",
                bad.to_bits()
            );
        }
    }

    #[test]
    fn first_non_finite_index_is_reported() {
        let err = read_pcm_f32le(&bytes(&[f32::INFINITY, 0.0, f32::NAN])[..]).unwrap_err();
        assert!(matches!(err, PcmError::NonFinite { index: 0 }), "{err}");
        let mut input = vec![0.0f32; 100_000];
        input[99_999] = f32::NEG_INFINITY;
        let err = read_pcm_f32le(&bytes(&input)[..]).unwrap_err();
        assert!(
            matches!(err, PcmError::NonFinite { index: 99_999 }),
            "{err}"
        );
    }

    #[test]
    fn non_finite_sample_split_across_reads_is_rejected() {
        let err = read_pcm_f32le(chunked(bytes(&[0.0, f32::NAN]), vec![3])).unwrap_err();
        assert!(matches!(err, PcmError::NonFinite { index: 1 }), "{err}");
    }

    #[test]
    fn single_byte_reads_over_many_samples() {
        let input: Vec<f32> = (0..10_000).map(|i| (i as f32 * 0.37).sin()).collect();
        let pcm = read_pcm_f32le(chunked(bytes(&input), vec![1])).unwrap();
        assert_eq!(bits(&pcm.0), bits(&input));
    }

    #[test]
    fn reads_of_one_to_three_bytes_in_every_rotation() {
        let input: Vec<f32> = (0..257).map(|i| i as f32 - 128.0).collect();
        for sizes in [
            vec![1, 2, 3],
            vec![2, 3, 1],
            vec![3, 1, 2],
            vec![3, 3, 3, 1],
            vec![2, 2, 1],
        ] {
            let pcm = read_pcm_f32le(chunked(bytes(&input), sizes.clone())).unwrap();
            assert_eq!(pcm.0, input, "sizes {sizes:?}");
        }
    }

    #[test]
    fn exactly_one_chunk_and_chunk_boundaries() {
        for samples in [CHUNK / 4 - 1, CHUNK / 4, CHUNK / 4 + 1, CHUNK / 2] {
            let input: Vec<f32> = (0..samples).map(|i| i as f32).collect();
            assert_eq!(
                read_pcm_f32le(&bytes(&input)[..]).unwrap().0.len(),
                samples,
                "samples {samples}"
            );
        }
    }

    #[test]
    fn large_input_in_unaligned_reads() {
        // ~1 minute of audio, delivered in prime-sized (unaligned) reads.
        let n = 60 * 16_000;
        let input: Vec<f32> = (0..n)
            .map(|i| ((i % 2000) as f32 - 1000.0) / 1000.0)
            .collect();
        let pcm = read_pcm_f32le(chunked(bytes(&input), vec![4093, 7, 65_537])).unwrap();
        assert_eq!(pcm.0.len(), n);
        assert_eq!(bits(&pcm.0), bits(&input));
    }

    #[test]
    fn io_error_after_partial_data_is_reported() {
        struct FailMidway(usize);
        impl Read for FailMidway {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.0 == 0 {
                    return Err(io::Error::other("pipe closed"));
                }
                let n = self.0.min(buf.len()).min(6);
                buf[..n].fill(0);
                self.0 -= n;
                Ok(n)
            }
        }
        assert!(matches!(
            read_pcm_f32le(FailMidway(10)),
            Err(PcmError::Io(_))
        ));
    }

    #[test]
    fn repeated_interrupts_mid_sample_are_retried() {
        /// Returns 1 byte, then Interrupted, alternating.
        struct Flaky {
            data: Vec<u8>,
            pos: usize,
            interrupt: bool,
        }
        impl Read for Flaky {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.interrupt = !self.interrupt;
                if self.interrupt {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                if self.pos == self.data.len() || buf.is_empty() {
                    return Ok(0);
                }
                buf[0] = self.data[self.pos];
                self.pos += 1;
                Ok(1)
            }
        }
        let input = [0.125f32, -0.75];
        let pcm = read_pcm_f32le(Flaky {
            data: bytes(&input),
            pos: 0,
            interrupt: false,
        })
        .unwrap();
        assert_eq!(pcm.0, input);
    }

    #[test]
    fn duration_matches_sample_count() {
        let pcm = read_pcm_f32le(&vec![0u8; 16_000 * 4 * 3][..]).unwrap();
        assert_eq!(pcm.duration(), std::time::Duration::from_secs(3));
    }
}
