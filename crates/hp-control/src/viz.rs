//! The viz channel's wire format: what a `subscribe_viz` asks for, and the
//! binary frame that comes back on the per-subscriber pipe.
//!
//! Binary rather than JSON because 32 bands at 60 Hz as JSON floats is a
//! quarter megabyte a second of number formatting for data that is natively
//! 32 bytes; a separate pipe rather than interleaving because mixing framed
//! binary with newline-delimited JSON is a parsing hazard for every client
//! that will ever be written (`docs/control-api.md`). Unstable until v1.0.
//!
//! Frame layout, little-endian:
//!
//! ```text
//! offset  size  field
//! 0       4     magic          the ASCII bytes "HPV1"
//! 4       8     timestamp_us   microseconds since the UNIX epoch, source clock,
//!                              taken when the analyser was read
//! 12      1     n_bands
//! 13      1     depth          0 = u8, 1 = f32
//! 14      1     flags          bit0 = beat detected
//! 15      1     reserved       0
//! 16      1     level_peak     0..255
//! 17      1     level_rms      0..255
//! 18      n     spectrum       n_bands x (1 or 4 bytes), 0..1 full scale
//! ```
//!
//! The header is fixed and self-describing, so a client can resync on the
//! magic after a dropped connection and a frame's length is known from its
//! first fourteen bytes.

use std::fmt;

/// The four bytes every frame starts with, in wire order. As a little-endian
/// `u32` that reads `0x31565048`; `control-api.md` names the bytes.
pub const MAGIC: [u8; 4] = *b"HPV1";
/// Bytes before the spectrum.
pub const HEADER_LEN: usize = 18;
/// `flags` bit: a beat was detected in this frame.
pub const FLAG_BEAT: u8 = 0b0000_0001;

/// The rates a subscriber may ask for. The source captures at the highest
/// rate any subscriber wants and the others take every second or fourth
/// frame, so the set is deliberately 15, 30, 60 and nothing between.
pub const RATES_HZ: [u32; 3] = [15, 30, 60];
pub const MIN_BANDS: u8 = 8;
pub const MAX_BANDS: u8 = 128;

/// The name of subscriber `id`'s pipe. A Windows name, ungated for the same
/// reason as `PIPE_NAME`; the POSIX paths in control-api.md land when a port
/// does.
pub fn pipe_name(id: u32) -> String {
    format!(r"\\.\pipe\hurricane-party-viz-{id:04x}")
}

/// How each spectrum value is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Depth {
    /// One byte, 0..255. LED-friendly.
    #[default]
    U8 = 0,
    /// A little-endian `f32`, 0..1.
    F32 = 1,
}

impl Depth {
    pub fn bytes_per_band(self) -> usize {
        match self {
            Depth::U8 => 1,
            Depth::F32 => 4,
        }
    }
    pub fn from_wire(b: u8) -> Option<Depth> {
        match b {
            0 => Some(Depth::U8),
            1 => Some(Depth::F32),
            _ => None,
        }
    }
    pub fn parse(s: &str) -> Option<Depth> {
        match s {
            "u8" => Some(Depth::U8),
            "f32" => Some(Depth::F32),
            _ => None,
        }
    }
}

/// Which parts of a frame carry data. The header is always the full
/// eighteen bytes; leaving something out zeroes it (or, for the spectrum,
/// sends `n_bands = 0`) rather than reshaping the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Include {
    pub spectrum: bool,
    pub level: bool,
    pub beat: bool,
}

impl Default for Include {
    fn default() -> Self {
        Self {
            spectrum: true,
            level: true,
            beat: true,
        }
    }
}

impl Include {
    /// From the request's list. Unknown names are an error, so a typo does
    /// not silently produce a frame with nothing in it.
    pub fn parse(names: &[String]) -> Result<Include, String> {
        let mut inc = Include {
            spectrum: false,
            level: false,
            beat: false,
        };
        for n in names {
            match n.as_str() {
                "spectrum" => inc.spectrum = true,
                "level" => inc.level = true,
                "beat" => inc.beat = true,
                other => return Err(format!("unknown include {other:?}")),
            }
        }
        Ok(inc)
    }
}

/// What a `subscribe_viz` asked for, validated.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VizParams {
    pub bands: u8,
    pub rate_hz: u32,
    pub depth: Depth,
    pub include: Include,
}

impl Default for VizParams {
    /// The LED wall's numbers from control-api.md.
    fn default() -> Self {
        Self {
            bands: 32,
            rate_hz: 30,
            depth: Depth::U8,
            include: Include::default(),
        }
    }
}

/// One frame, decoded or about to be encoded. `spectrum` is 0..1 full scale
/// whatever the depth; the depth only decides how it is written.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub timestamp_us: u64,
    pub depth: Depth,
    pub beat: bool,
    pub level_peak: u8,
    pub level_rms: u8,
    pub spectrum: Vec<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Not enough bytes yet; the value is how many a whole frame needs, when
    /// the header is complete, or `HEADER_LEN` when it is not.
    Incomplete(usize),
    BadMagic,
    BadDepth(u8),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Incomplete(n) => write!(f, "need {n} bytes for a frame"),
            DecodeError::BadMagic => write!(f, "not a viz frame (bad magic)"),
            DecodeError::BadDepth(d) => write!(f, "unknown depth {d}"),
        }
    }
}

impl std::error::Error for DecodeError {}

impl Frame {
    pub fn encoded_len(&self) -> usize {
        HEADER_LEN + self.spectrum.len() * self.depth.bytes_per_band()
    }

    /// Write the frame into `out`, replacing its contents. Reusing one buffer
    /// per subscriber keeps the 60 Hz path allocation-free.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.clear();
        out.reserve(self.encoded_len());
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&self.timestamp_us.to_le_bytes());
        out.push(self.spectrum.len().min(MAX_BANDS as usize) as u8);
        out.push(self.depth as u8);
        out.push(if self.beat { FLAG_BEAT } else { 0 });
        out.push(0);
        out.push(self.level_peak);
        out.push(self.level_rms);
        for &v in self.spectrum.iter().take(MAX_BANDS as usize) {
            let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
            match self.depth {
                Depth::U8 => out.push((v * 255.0).round() as u8),
                Depth::F32 => out.extend_from_slice(&v.to_le_bytes()),
            }
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.encoded_len());
        self.encode_into(&mut out);
        out
    }

    /// Parse one frame from the front of `buf`. Returns it with the number of
    /// bytes it used, so a client can walk a stream of them.
    pub fn decode(buf: &[u8]) -> Result<(Frame, usize), DecodeError> {
        if buf.len() < HEADER_LEN {
            return Err(DecodeError::Incomplete(HEADER_LEN));
        }
        if buf[..4] != MAGIC {
            return Err(DecodeError::BadMagic);
        }
        let mut ts = [0u8; 8];
        ts.copy_from_slice(&buf[4..12]);
        let n = buf[12] as usize;
        let depth = Depth::from_wire(buf[13]).ok_or(DecodeError::BadDepth(buf[13]))?;
        let total = HEADER_LEN + n * depth.bytes_per_band();
        if buf.len() < total {
            return Err(DecodeError::Incomplete(total));
        }
        let body = &buf[HEADER_LEN..total];
        let spectrum = match depth {
            Depth::U8 => body.iter().map(|&b| b as f32 / 255.0).collect(),
            Depth::F32 => body
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect(),
        };
        Ok((
            Frame {
                timestamp_us: u64::from_le_bytes(ts),
                depth,
                beat: buf[14] & FLAG_BEAT != 0,
                level_peak: buf[16],
                level_rms: buf[17],
                spectrum,
            },
            total,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(depth: Depth) -> Frame {
        Frame {
            timestamp_us: 0x0102_0304_0506_0708,
            depth,
            beat: true,
            level_peak: 200,
            level_rms: 90,
            spectrum: vec![0.0, 0.5, 1.0],
        }
    }

    /// The layout in control-api.md, byte for byte. A client written from
    /// the doc has to read what this writes.
    #[test]
    fn header_layout_is_the_documented_one() {
        let b = sample(Depth::U8).encode();
        assert_eq!(&b[..4], b"HPV1");
        assert_eq!(&b[4..12], &[8, 7, 6, 5, 4, 3, 2, 1]); // little-endian
        assert_eq!(b[12], 3); // n_bands
        assert_eq!(b[13], 0); // u8
        assert_eq!(b[14], FLAG_BEAT);
        assert_eq!(b[15], 0); // reserved
        assert_eq!(b[16], 200);
        assert_eq!(b[17], 90);
        assert_eq!(&b[18..], &[0, 128, 255]);
        assert_eq!(b.len(), HEADER_LEN + 3);
    }

    #[test]
    fn f32_bands_are_four_bytes_each() {
        let b = sample(Depth::F32).encode();
        assert_eq!(b[13], 1);
        assert_eq!(b.len(), HEADER_LEN + 12);
        assert_eq!(&b[22..26], &0.5f32.to_le_bytes());
    }

    #[test]
    fn round_trips_at_both_depths() {
        for depth in [Depth::U8, Depth::F32] {
            let f = sample(depth);
            let (back, used) = Frame::decode(&f.encode()).unwrap();
            assert_eq!(used, f.encoded_len());
            assert_eq!(back.timestamp_us, f.timestamp_us);
            assert_eq!(back.depth, depth);
            assert!(back.beat);
            assert_eq!((back.level_peak, back.level_rms), (200, 90));
            for (a, b) in back.spectrum.iter().zip(&f.spectrum) {
                assert!((a - b).abs() < 1.0 / 255.0, "{a} vs {b}");
            }
        }
    }

    /// A stream is frames back to back; decode says how far to advance.
    #[test]
    fn decode_walks_a_stream() {
        let mut stream = sample(Depth::U8).encode();
        let mut second = sample(Depth::F32);
        second.timestamp_us = 99;
        stream.extend(second.encode());
        let (a, n) = Frame::decode(&stream).unwrap();
        let (b, m) = Frame::decode(&stream[n..]).unwrap();
        assert_eq!(a.timestamp_us, 0x0102_0304_0506_0708);
        assert_eq!(b.timestamp_us, 99);
        assert_eq!(n + m, stream.len());
    }

    #[test]
    fn incomplete_says_how_much_it_needs() {
        let b = sample(Depth::U8).encode();
        assert_eq!(
            Frame::decode(&b[..5]),
            Err(DecodeError::Incomplete(HEADER_LEN))
        );
        assert_eq!(Frame::decode(&b[..19]), Err(DecodeError::Incomplete(21)));
        assert!(matches!(
            Frame::decode(b"nope, not a frame at all"),
            Err(DecodeError::BadMagic)
        ));
    }

    #[test]
    fn out_of_range_values_are_clamped_not_wrapped() {
        let f = Frame {
            spectrum: vec![-1.0, 2.0, f32::NAN],
            ..sample(Depth::U8)
        };
        assert_eq!(&f.encode()[18..], &[0, 255, 0]);
    }

    #[test]
    fn include_rejects_a_typo() {
        assert!(Include::parse(&["spectrum".into(), "beet".into()]).is_err());
        let i = Include::parse(&["level".into()]).unwrap();
        assert_eq!(
            i,
            Include {
                spectrum: false,
                level: true,
                beat: false
            }
        );
    }

    #[test]
    fn pipe_names_are_per_subscriber() {
        assert_eq!(pipe_name(0x7f3a), r"\\.\pipe\hurricane-party-viz-7f3a");
        assert_ne!(pipe_name(1), pipe_name(2));
    }
}
