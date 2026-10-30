use crate::classify::RequestClass;
use browser_core::{Clock, RequestId, TabId};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Disposition {
    Allowed,
    Held,
    Released,
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct RequestRecord {
    pub id: RequestId,
    pub tab: TabId,
    pub at: Instant,
    pub url: String,
    pub class: RequestClass,
    pub disposition: Disposition,
}

/// Bounded per-tab request log plus counters. Capacity-limited so monitoring can
/// never become a memory leak of its own.
pub struct NetworkMonitor {
    clock: Arc<dyn Clock>,
    log: VecDeque<RequestRecord>,
    counts: HashMap<(RequestClass, Disposition), u64>,
}

impl NetworkMonitor {
    pub const CAPACITY: usize = 256;

    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock, log: VecDeque::with_capacity(Self::CAPACITY), counts: HashMap::new() }
    }

    pub(crate) fn record(&mut self, id: RequestId, tab: TabId, url: &Url, class: RequestClass, d: Disposition) {
        if self.log.len() == Self::CAPACITY {
            self.log.pop_front();
        }
        // Query strings often carry tokens: log scheme+host+path only.
        let mut shown = url.clone();
        shown.set_query(None);
        shown.set_fragment(None);
        self.log.push_back(RequestRecord { id, tab, at: self.clock.now(), url: shown.to_string(), class, disposition: d });
        *self.counts.entry((class, d)).or_default() += 1;
    }

    pub(crate) fn set_disposition(&mut self, id: RequestId, d: Disposition) {
        if let Some(r) = self.log.iter_mut().find(|r| r.id == id) {
            r.disposition = d;
        }
        // Count the transition by class.
        if let Some(class) = self.log.iter().find(|r| r.id == id).map(|r| r.class) {
            *self.counts.entry((class, d)).or_default() += 1;
        }
    }

    pub fn count(&self, class: RequestClass, d: Disposition) -> u64 {
        self.counts.get(&(class, d)).copied().unwrap_or(0)
    }

    /// Requests that actually left the browser (allowed or later released) within `window`.
    pub fn sent_within(&self, window: Duration) -> usize {
        let now = self.clock.now();
        self.log
            .iter()
            .filter(|r| matches!(r.disposition, Disposition::Allowed | Disposition::Released) && now.duration_since(r.at) <= window)
            .count()
    }

    pub fn recent(&self) -> impl Iterator<Item = &RequestRecord> {
        self.log.iter()
    }
}
