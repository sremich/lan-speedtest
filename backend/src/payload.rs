//! Zero-copy download payload.
//!
//! The engine only cares that `GET /__down?bytes=N` yields N bytes; the
//! content is discarded. So we allocate one shared buffer at startup and
//! hand out refcounted slices of it. No per-request allocation, no copying,
//! and nothing for the allocator to do while a 10 GbE client is pulling.
//!
//! `size_hint` reports the exact remaining length, which lets hyper send a
//! real `content-length` and skip chunked framing entirely.

use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use http_body::{Body, Frame, SizeHint};

/// The repeating source buffer, allocated once and shared by every request.
#[derive(Clone, Debug)]
pub struct PayloadSource {
    chunk: Bytes,
}

impl PayloadSource {
    /// Build a source buffer of `chunk_bytes`, filled with printable ASCII.
    ///
    /// Content is arbitrary but deliberately not all-zero: an incompressible-
    /// looking pattern keeps any transparent compression on the path from
    /// flattering the result. We never advertise `content-encoding`, and the
    /// engine's own upload payload is all-zero, so this only guards the
    /// download direction.
    pub fn new(chunk_bytes: usize) -> Self {
        let chunk_bytes = chunk_bytes.max(1);
        let mut buf = Vec::with_capacity(chunk_bytes);
        // Cheap deterministic byte pattern over the printable ASCII range.
        let mut x: u32 = 0x9E37_79B9;
        for _ in 0..chunk_bytes {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            buf.push(b'!' + (x >> 24) as u8 % 94);
        }
        Self {
            chunk: Bytes::from(buf),
        }
    }

    pub fn chunk_len(&self) -> usize {
        self.chunk.len()
    }

    /// A body that emits exactly `total` bytes as slices of the shared buffer.
    pub fn body(&self, total: u64) -> RepeatBody {
        RepeatBody {
            chunk: self.chunk.clone(),
            remaining: total,
        }
    }
}

/// Emits `remaining` bytes by repeatedly slicing one shared buffer.
#[derive(Debug)]
pub struct RepeatBody {
    chunk: Bytes,
    remaining: u64,
}

impl Body for RepeatBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        if this.remaining == 0 {
            return Poll::Ready(None);
        }
        let take = this.remaining.min(this.chunk.len() as u64) as usize;
        // Bytes::slice is a refcount bump — no copy of the payload itself.
        let frame = this.chunk.slice(0..take);
        this.remaining -= take as u64;
        Poll::Ready(Some(Ok(Frame::data(frame))))
    }

    fn is_end_stream(&self) -> bool {
        self.remaining == 0
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.remaining)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    async fn collect_len(body: RepeatBody) -> usize {
        body.collect().await.unwrap().to_bytes().len()
    }

    #[tokio::test]
    async fn emits_exactly_the_requested_length() {
        let src = PayloadSource::new(1024);
        for n in [0u64, 1, 1023, 1024, 1025, 4096, 100_000] {
            assert_eq!(collect_len(src.body(n)).await, n as usize, "n = {n}");
        }
    }

    #[test]
    fn size_hint_is_exact_so_hyper_can_set_content_length() {
        let src = PayloadSource::new(64);
        let body = src.body(4096);
        assert_eq!(body.size_hint().exact(), Some(4096));
    }

    #[test]
    fn zero_length_body_is_immediately_end_of_stream() {
        let src = PayloadSource::new(64);
        assert!(src.body(0).is_end_stream());
        assert!(!src.body(1).is_end_stream());
    }

    #[test]
    fn source_buffer_is_not_all_zero() {
        let src = PayloadSource::new(4096);
        assert!(src.chunk.iter().any(|&b| b != 0));
    }
}
