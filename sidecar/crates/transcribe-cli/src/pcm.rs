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
}
