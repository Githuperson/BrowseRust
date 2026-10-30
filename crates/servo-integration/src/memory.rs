//! Servo memory reports → per-tab attribution.
//!
//! Servo reports memory per *process* as `(path, kind, size)` triples.
//!
//! * `Explicit*` kinds are real allocations (Servo: "the single best number to
//!   focus on"). Where the path starts with `url(<document url>)` they are
//!   attributed to the tab showing that URL; the rest is engine-wide "shared"
//!   (WebRender, fonts, network).
//! * `NonExplicitSize` kinds are global or cross-cutting measures (`resident`,
//!   `vsize`, per-mapping `resident-according-to-smaps/…`). Adding them to the
//!   explicit figures double-counts, and `vsize` is virtual address space, not
//!   RAM. Only `resident` is kept, separately, as the engine's RSS.

use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct ServoMemory {
    /// Bytes attributed to a document URL (summed over frames with that URL).
    pub by_url: HashMap<String, u64>,
    /// Engine-wide explicit bytes not tied to any document.
    pub shared: u64,
    /// Sum of all explicit allocations (documents + shared).
    pub explicit_total: u64,
    /// The engine process's resident set size as Servo reports it (includes
    /// shared library pages; not unique RAM).
    pub resident: u64,
    /// Number of distinct reporting processes.
    pub processes: usize,
    /// The largest non-document reports, for diagnostics (`path`, bytes).
    pub largest_shared: Vec<(String, u64)>,
}

/// One flattened report line: the path and its size.
pub struct Line<'a> {
    pub path: &'a [String],
    pub size: u64,
    /// `true` for the `Explicit*` report kinds.
    pub explicit: bool,
}

/// Extract the document URL from a path segment like `url(https://a.test/)`.
fn url_of(segment: &str) -> Option<&str> {
    let inner = segment.strip_prefix("url(")?;
    let end = inner.rfind(')')?;
    // Some reporters append " (id)" or similar after the URL; keep the URL part.
    Some(inner[..end].split(", ").next().unwrap_or(&inner[..end]))
}

pub fn attribute<'a>(processes: usize, lines: impl Iterator<Item = Line<'a>>) -> ServoMemory {
    let mut m = ServoMemory { processes, ..Default::default() };
    let mut shared_lines: Vec<(String, u64)> = Vec::new();
    for l in lines {
        if !l.explicit {
            if l.path.len() == 1 && l.path[0] == "resident" {
                m.resident += l.size;
            }
            continue;
        }
        m.explicit_total += l.size;
        match l.path.first().and_then(|s| url_of(s)) {
            Some(u) => *m.by_url.entry(u.to_string()).or_default() += l.size,
            None => {
                m.shared += l.size;
                shared_lines.push((l.path.join("/"), l.size));
            }
        }
    }
    shared_lines.sort_by(|a, b| b.1.cmp(&a.1));
    shared_lines.truncate(8);
    m.largest_shared = shared_lines;
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn attributes_url_paths_to_documents_and_the_rest_to_shared() {
        let a = p(&["url(https://a.test/)", "js", "gc-heap"]);
        let a2 = p(&["url(https://a.test/)", "dom-tree"]);
        let b = p(&["url(https://b.test/x?y=(1))", "layout"]);
        let w = p(&["webrender", "display-list"]);
        let vsize = p(&["vsize"]);
        let resident = p(&["resident"]);
        let lines = vec![
            Line { path: &a, size: 100, explicit: true },
            Line { path: &a2, size: 50, explicit: true },
            Line { path: &b, size: 7, explicit: true },
            Line { path: &w, size: 1000, explicit: true },
            // Process-level metrics must not leak into the explicit totals.
            Line { path: &vsize, size: 5_000_000, explicit: false },
            Line { path: &resident, size: 300, explicit: false },
        ];
        let m = attribute(2, lines.into_iter());
        assert_eq!(m.by_url["https://a.test/"], 150);
        assert_eq!(m.by_url["https://b.test/x?y=(1)"], 7, "nested parentheses survive");
        assert_eq!(m.shared, 1000);
        assert_eq!(m.explicit_total, 1157, "vsize/resident are excluded");
        assert_eq!(m.resident, 300);
        assert_eq!(m.processes, 2);
    }
}
