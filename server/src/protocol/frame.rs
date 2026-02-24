//! Length-prefixed frame codec over TCP (4 bytes LE length, then payload).
//! Uses tokio_util's LengthDelimitedCodec for mature, well-tested framing.

use tokio_util::codec::length_delimited::LengthDelimitedCodec;

/// Maximum allowed frame body length (64 KiB); sufficient for game state/input blobs.
pub const MAX_FRAME_LEN: usize = 64 * 1024;

/// Builds a length-delimited codec: 4-byte little-endian length prefix, then payload.
/// Use with `Framed::new(stream, frame_codec())`.
pub fn frame_codec() -> LengthDelimitedCodec {
    LengthDelimitedCodec::builder()
        .length_field_length(4)
        .little_endian()
        .max_frame_length(MAX_FRAME_LEN)
        .new_codec()
}
