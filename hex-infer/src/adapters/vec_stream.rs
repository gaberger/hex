//! A `futures_stream::Stream` over an already-collected `Vec<StreamChunk>`.
//!
//! `hex_core::ports::inference` defines its own minimal `Stream` trait rather
//! than depending on `futures`, so adapters that cannot stream incrementally
//! need a tiny adapter to satisfy `IInferencePort::stream`. Providers that do
//! stream for real (Ollama) use an MPSC-backed stream instead; this one is for
//! providers that must buffer the whole response first.

use hex_core::ports::inference::{futures_stream, StreamChunk};

/// Yields pre-collected chunks in order, then `None`.
pub(crate) struct VecStream {
    inner: std::vec::IntoIter<StreamChunk>,
}

impl VecStream {
    pub(crate) fn new(chunks: Vec<StreamChunk>) -> Self {
        Self {
            inner: chunks.into_iter(),
        }
    }
}

impl futures_stream::Stream for VecStream {
    type Item = StreamChunk;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Ready(self.inner.next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::domain::messages::StopReason;
    use hex_core::ports::inference::futures_stream::Stream as _;
    use std::task::{Context, Poll};

    /// A `Waker` that does nothing — `VecStream` never parks, so polling it
    /// needs a context but never uses one.
    fn noop_context() -> Context<'static> {
        Context::from_waker(std::task::Waker::noop())
    }

    #[test]
    fn yields_every_chunk_in_order_then_ends() {
        let mut s = VecStream::new(vec![
            StreamChunk::TextDelta("a".into()),
            StreamChunk::TextDelta("b".into()),
            StreamChunk::MessageStop(StopReason::EndTurn),
        ]);
        let mut cx = noop_context();
        let mut seen = Vec::new();
        loop {
            match std::pin::Pin::new(&mut s).poll_next(&mut cx) {
                Poll::Ready(Some(c)) => seen.push(format!("{:?}", c)),
                Poll::Ready(None) => break,
                Poll::Pending => unreachable!("VecStream never parks"),
            }
        }
        assert_eq!(seen.len(), 3);
        assert!(seen[0].contains('a'));
        assert!(seen[1].contains('b'));
        assert!(seen[2].contains("MessageStop"));
    }

    #[test]
    fn empty_stream_ends_immediately() {
        let mut s = VecStream::new(vec![]);
        let mut cx = noop_context();
        assert!(matches!(
            std::pin::Pin::new(&mut s).poll_next(&mut cx),
            Poll::Ready(None)
        ));
    }
}
