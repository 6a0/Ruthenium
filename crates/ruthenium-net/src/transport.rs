//! The byte stream under the packets (`network/Connection.java`'s pipeline): AES/CFB8
//! once the login turns it on, VarInt length frames, and zlib compression once the
//! server sets a threshold.
//!
//! With compression, a frame holds the uncompressed length as a VarInt (0: the packet
//! follows as it is) and then the packet, zlib-compressed. Vanilla's client accepts
//! packets compressed below the threshold (`validateDecompressed` is off) but checks
//! that the stated length is right and at most 8 MiB (`CompressionDecoder.java`).

use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};
use ruthenium_protocol::wire::{self, WireError};

use crate::crypt::Cfb8;

/// The largest packet a compressed frame may expand to
/// (`CompressionDecoder.MAXIMUM_UNCOMPRESSED_LENGTH`).
pub const MAX_UNCOMPRESSED: usize = 8 << 20;

/// Frames, compresses and encrypts in one direction, and the reverse in the other.
#[derive(Debug, Default)]
pub struct Transport {
    threshold: Option<usize>,
    encrypt: Option<Cfb8>,
    decrypt: Option<Cfb8>,
    /// Received bytes, decrypted, not yet split into frames.
    inbound: Vec<u8>,
}

impl Transport {
    pub fn new() -> Transport {
        Transport::default()
    }

    /// Compression from now on: packets of `threshold` bytes or more are compressed;
    /// a negative threshold turns it off (`Connection.setupCompression`).
    pub fn set_compression(&mut self, threshold: i32) {
        self.threshold = usize::try_from(threshold).ok();
    }

    /// Encryption from now on, both directions. Bytes already received are not
    /// decrypted: the caller turns it on before passing more in.
    pub fn set_encryption(&mut self, secret: &[u8; 16]) {
        self.encrypt = Some(Cfb8::new(secret));
        self.decrypt = Some(Cfb8::new(secret));
    }

    /// A packet's bytes (id and fields) as they go on the wire.
    pub fn encode(&mut self, packet: &[u8]) -> Result<Vec<u8>, WireError> {
        let mut body = Vec::with_capacity(packet.len() + 5);
        match self.threshold {
            None => body.extend_from_slice(packet),
            Some(t) if packet.len() < t => {
                wire::write_varint(&mut body, 0);
                body.extend_from_slice(packet);
            }
            Some(_) => {
                if packet.len() > MAX_UNCOMPRESSED {
                    return Err(WireError::TooLong {
                        len: packet.len(),
                        max: MAX_UNCOMPRESSED,
                    });
                }
                wire::write_varint(&mut body, packet.len() as i32);
                deflate(packet, &mut body);
            }
        }
        if body.len() > wire::MAX_FRAME {
            return Err(WireError::TooLong {
                len: body.len(),
                max: wire::MAX_FRAME,
            });
        }
        let mut out = Vec::with_capacity(body.len() + 3);
        wire::write_varint(&mut out, body.len() as i32);
        out.extend_from_slice(&body);
        if let Some(c) = &mut self.encrypt {
            c.encrypt(&mut out);
        }
        Ok(out)
    }

    /// Takes in received bytes.
    pub fn receive(&mut self, bytes: &[u8]) {
        let start = self.inbound.len();
        self.inbound.extend_from_slice(bytes);
        if let Some(c) = &mut self.decrypt {
            c.decrypt(&mut self.inbound[start..]);
        }
    }

    /// The next whole packet received (id and fields), if there is one.
    pub fn next_packet(&mut self) -> Result<Option<Vec<u8>>, WireError> {
        let Some((frame, used)) = wire::split_frame(&self.inbound)? else {
            return Ok(None);
        };
        // `Varint21FrameDecoder` refuses empty frames.
        if frame.is_empty() {
            return Err(WireError::Invalid("empty frame"));
        }
        let packet = match self.threshold {
            None => frame.to_vec(),
            Some(_) => {
                let mut r = wire::Reader::new(frame);
                let len = r.varint()?;
                let data = r.rest();
                if len == 0 {
                    data.to_vec()
                } else {
                    let len = usize::try_from(len).map_err(|_| WireError::NegativeLength(len))?;
                    if len > MAX_UNCOMPRESSED {
                        return Err(WireError::TooLong {
                            len,
                            max: MAX_UNCOMPRESSED,
                        });
                    }
                    inflate(data, len)?
                }
            }
        };
        self.inbound.drain(..used);
        Ok(Some(packet))
    }

    /// Bytes received but not yet a whole frame.
    pub fn buffered(&self) -> usize {
        self.inbound.len()
    }
}

/// zlib at Java's `Deflater` default level (6).
fn deflate(data: &[u8], out: &mut Vec<u8>) {
    let mut c = Compress::new(Compression::new(6), true);
    out.reserve(data.len() / 2 + 64);
    loop {
        let before = c.total_in() as usize;
        let status = c
            .compress_vec(&data[before..], out, FlushCompress::Finish)
            .expect("deflate into a growing buffer");
        if status == Status::StreamEnd {
            return;
        }
        out.reserve(out.capacity().max(256));
    }
}

/// Inflates `data`, which must expand to exactly `len` bytes.
fn inflate(data: &[u8], len: usize) -> Result<Vec<u8>, WireError> {
    let mut d = Decompress::new(true);
    // One more byte than stated, to catch a packet that expands past it.
    let mut out = Vec::with_capacity(len + 1);
    let status = d
        .decompress_vec(data, &mut out, FlushDecompress::Finish)
        .map_err(|_| WireError::Invalid("compressed packet"))?;
    if status != Status::StreamEnd || out.len() != len {
        return Err(WireError::Invalid("compressed packet length"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Transport, Transport) {
        (Transport::new(), Transport::new())
    }

    fn roundtrip(a: &mut Transport, b: &mut Transport, packet: &[u8]) {
        let bytes = a.encode(packet).unwrap();
        // Arriving in pieces changes nothing.
        let (x, y) = bytes.split_at(bytes.len() / 2);
        b.receive(x);
        if !y.is_empty() {
            assert!(b.next_packet().unwrap().is_none() || x.len() == bytes.len());
        }
        b.receive(y);
        assert_eq!(b.next_packet().unwrap().as_deref(), Some(packet));
        assert_eq!(b.buffered(), 0);
    }

    #[test]
    fn plain_compressed_and_encrypted() {
        let (mut a, mut b) = pair();
        roundtrip(&mut a, &mut b, &[0, 1, 2, 3]);
        a.set_compression(64);
        b.set_compression(64);
        let small = [1u8; 10];
        let bytes = a.encode(&small).unwrap();
        assert_eq!(&bytes[..2], [11, 0]);
        b.receive(&bytes);
        assert_eq!(b.next_packet().unwrap().unwrap(), small);
        let big: Vec<u8> = (0..5000u32).map(|i| (i % 7) as u8).collect();
        let bytes = a.encode(&big).unwrap();
        assert!(bytes.len() < 200, "{}", bytes.len());
        b.receive(&bytes);
        assert_eq!(b.next_packet().unwrap().unwrap(), big);
        a.set_encryption(&[9; 16]);
        b.set_encryption(&[9; 16]);
        roundtrip(&mut a, &mut b, &big);
        roundtrip(&mut a, &mut b, &[5, 6]);
        a.set_compression(-1);
        b.set_compression(-1);
        roundtrip(&mut a, &mut b, &big[..300]);
    }

    #[test]
    fn refuses_wrong_lengths_and_empty_frames() {
        let mut b = Transport::new();
        b.receive(&[0]);
        assert!(b.next_packet().is_err());
        let mut b = Transport::new();
        b.set_compression(1);
        // States 100 bytes, inflates to 3.
        let mut z = Vec::new();
        deflate(&[1, 2, 3], &mut z);
        let mut body = Vec::new();
        wire::write_varint(&mut body, 100);
        body.extend_from_slice(&z);
        let mut frame = Vec::new();
        wire::write_varint(&mut frame, body.len() as i32);
        frame.extend_from_slice(&body);
        b.receive(&frame);
        assert!(b.next_packet().is_err());
    }
}
