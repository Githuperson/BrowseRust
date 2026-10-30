//! Bounded buffering and the suspension plan for streaming protocols.
//!
//! # Protocol limits — read before trusting "paused"
//!
//! * **WebSocket** has no application-level pause. If the embedder simply stops
//!   *reading* the socket, TCP flow control eventually stalls the server's sends,
//!   but the server may keep buffering or may time the connection out (ping/pong).
//!   We therefore never promise "zero incoming bytes".
//! * **SSE** (`text/event-stream`) is a long-lived HTTP response. Same TCP-level
//!   caveats; additionally the browser's `EventSource` reconnects on its own.
//!
//! The strategy, in order of preference ([`SuspensionPlan`]):
//! 1. `PauseDelivery` — keep the socket, stop *delivering* to JS, and hold a
//!    strictly bounded queue ([`BoundedQueue`]);
//! 2. when the queue overflows, `CloseAndReconnect` — close cleanly and let the
//!    page reconnect (new connection attempts are themselves held by the
//!    request controller until the tab is foregrounded).

use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuspensionPlan {
    /// Keep the connection; queue (bounded) instead of delivering to script.
    PauseDelivery,
    /// Close the connection and reconnect on resume.
    CloseAndReconnect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    WebSocket,
    ServerSentEvents,
}

#[derive(Clone, Copy, Debug)]
pub struct StreamLimits {
    pub max_messages: usize,
    pub max_bytes: usize,
}

impl Default for StreamLimits {
    fn default() -> Self {
        Self { max_messages: 256, max_bytes: 256 * 1024 }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Push {
    Queued,
    /// The queue is full. The caller must apply `CloseAndReconnect`; nothing was stored.
    Overflow,
}

/// A queue with hard message *and* byte caps. Never grows past its limits.
#[derive(Debug)]
pub struct BoundedQueue {
    limits: StreamLimits,
    bytes: usize,
    msgs: VecDeque<Vec<u8>>,
    overflowed: bool,
}

impl BoundedQueue {
    pub fn new(limits: StreamLimits) -> Self {
        Self { limits, bytes: 0, msgs: VecDeque::new(), overflowed: false }
    }

    pub fn push(&mut self, msg: Vec<u8>) -> Push {
        if self.overflowed || self.msgs.len() >= self.limits.max_messages || self.bytes + msg.len() > self.limits.max_bytes {
            self.overflowed = true;
            return Push::Overflow;
        }
        self.bytes += msg.len();
        self.msgs.push_back(msg);
        Push::Queued
    }

    pub fn len(&self) -> usize {
        self.msgs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.msgs.is_empty()
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// Drain for in-order delivery on resume.
    pub fn drain(&mut self) -> Vec<Vec<u8>> {
        self.bytes = 0;
        self.msgs.drain(..).collect()
    }
}

/// Decide the plan for a stream. If the embedder cannot defer delivery to
/// script (`can_pause_delivery == false`, true for stock Servo today), the only
/// honest option is close-and-reconnect.
pub fn plan_for(_protocol: Protocol, can_pause_delivery: bool) -> SuspensionPlan {
    if can_pause_delivery { SuspensionPlan::PauseDelivery } else { SuspensionPlan::CloseAndReconnect }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_never_exceeds_message_cap() {
        let mut q = BoundedQueue::new(StreamLimits { max_messages: 3, max_bytes: 1 << 20 });
        for _ in 0..3 {
            assert_eq!(q.push(vec![0; 10]), Push::Queued);
        }
        assert_eq!(q.push(vec![0; 10]), Push::Overflow);
        assert_eq!(q.len(), 3);
        assert!(q.overflowed());
    }

    #[test]
    fn queue_never_exceeds_byte_cap_and_stays_overflowed() {
        let mut q = BoundedQueue::new(StreamLimits { max_messages: 100, max_bytes: 100 });
        assert_eq!(q.push(vec![0; 60]), Push::Queued);
        assert_eq!(q.push(vec![0; 60]), Push::Overflow);
        // Once overflowed, even a small message is refused: ordering would be broken.
        assert_eq!(q.push(vec![0; 1]), Push::Overflow);
        assert_eq!(q.bytes(), 60);
    }

    #[test]
    fn drain_preserves_order_and_resets_accounting() {
        let mut q = BoundedQueue::new(StreamLimits::default());
        q.push(b"a".to_vec());
        q.push(b"b".to_vec());
        assert_eq!(q.drain(), vec![b"a".to_vec(), b"b".to_vec()]);
        assert_eq!(q.bytes(), 0);
        assert!(q.is_empty());
    }

    #[test]
    fn stock_engine_without_pause_support_falls_back_to_reconnect() {
        assert_eq!(plan_for(Protocol::WebSocket, false), SuspensionPlan::CloseAndReconnect);
        assert_eq!(plan_for(Protocol::ServerSentEvents, true), SuspensionPlan::PauseDelivery);
    }
}
