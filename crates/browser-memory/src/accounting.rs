use browser_core::{ByteSize, TabId};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    JsHeap,
    Images,
    Fonts,
    Layout,
    Gpu,
    Other,
}

impl Category {
    pub const ALL: [Category; 6] = [Self::JsHeap, Self::Images, Self::Fonts, Self::Layout, Self::Gpu, Self::Other];
}

/// One tab's attributed memory.
///
/// `private` is memory that disappears if the tab is discarded. `shared` is
/// memory the tab maps but that other tabs/the process also keep alive
/// (shared libraries, deduplicated resources, the font cache). Discarding a tab
/// frees **only** its private bytes, so the governor must reason about those,
/// and RSS must never be summed across tabs.
#[derive(Clone, Debug, Default)]
pub struct TabAccount {
    private: HashMap<Category, u64>,
    pub shared: ByteSize,
}

impl TabAccount {
    pub fn private_total(&self) -> ByteSize {
        ByteSize(self.private.values().sum())
    }
    pub fn private(&self, c: Category) -> ByteSize {
        ByteSize(self.private.get(&c).copied().unwrap_or(0))
    }
}

/// Per-tab ledger plus the process-wide measurement used as ground truth.
#[derive(Default)]
pub struct MemoryLedger {
    tabs: HashMap<TabId, TabAccount>,
    /// Process-wide unique footprint (PSS on Linux, `phys_footprint` on macOS).
    process_footprint: ByteSize,
}

impl MemoryLedger {
    pub fn set_private(&mut self, tab: TabId, c: Category, bytes: ByteSize) {
        self.tabs.entry(tab).or_default().private.insert(c, bytes.0);
    }
    pub fn set_shared(&mut self, tab: TabId, bytes: ByteSize) {
        self.tabs.entry(tab).or_default().shared = bytes;
    }
    pub fn set_process_footprint(&mut self, b: ByteSize) {
        self.process_footprint = b;
    }
    pub fn forget(&mut self, tab: TabId) {
        self.tabs.remove(&tab);
    }
    pub fn account(&self, tab: TabId) -> Option<&TabAccount> {
        self.tabs.get(&tab)
    }
    pub fn tab_private(&self, tab: TabId) -> ByteSize {
        self.tabs.get(&tab).map(TabAccount::private_total).unwrap_or_default()
    }

    /// Browser-wide usage: the measured process footprint when available (it
    /// already counts shared pages exactly once), otherwise the sum of private
    /// tab bytes as a lower bound.
    pub fn browser_total(&self) -> ByteSize {
        if self.process_footprint.0 > 0 {
            self.process_footprint
        } else {
            ByteSize(self.tabs.values().map(|t| t.private_total().0).sum())
        }
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_sums_by_category_and_overwrites_samples() {
        let mut l = MemoryLedger::default();
        let t = TabId(1);
        l.set_private(t, Category::JsHeap, ByteSize::mib(40));
        l.set_private(t, Category::Images, ByteSize::mib(10));
        l.set_private(t, Category::Images, ByteSize::mib(4));
        assert_eq!(l.tab_private(t), ByteSize::mib(44));
    }

    #[test]
    fn shared_memory_is_not_counted_as_private_or_double_counted() {
        let mut l = MemoryLedger::default();
        l.set_private(TabId(1), Category::JsHeap, ByteSize::mib(30));
        l.set_private(TabId(2), Category::JsHeap, ByteSize::mib(30));
        l.set_shared(TabId(1), ByteSize::mib(200));
        l.set_shared(TabId(2), ByteSize::mib(200));
        assert_eq!(l.browser_total(), ByteSize::mib(60), "fallback uses private only");
        l.set_process_footprint(ByteSize::mib(310));
        assert_eq!(l.browser_total(), ByteSize::mib(310), "measured footprint counts the 200 MiB once");
    }

    #[test]
    fn forgetting_a_tab_releases_its_account() {
        let mut l = MemoryLedger::default();
        l.set_private(TabId(1), Category::Other, ByteSize::mib(1));
        l.forget(TabId(1));
        assert_eq!(l.tab_count(), 0);
        assert_eq!(l.tab_private(TabId(1)), ByteSize(0));
    }
}
