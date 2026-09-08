//! Shared data model between the scanner and the UI.
//!
//! The tree is an arena: every node lives in `Tree::nodes` and refers to its
//! relatives by index. This keeps a multi-million-node scan compact and makes
//! the tree trivially `Send` so it can cross the scan thread boundary.

use std::path::{Path, PathBuf};

/// Index of a node inside [`Tree::nodes`].
pub type NodeId = usize;

/// What kind of filesystem entry a node describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Dir,
    File,
    /// Symlinks are reported but never followed.
    Symlink,
    /// Sockets, FIFOs, devices, and anything else.
    Other,
}

/// One filesystem entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// File name (no directory part). For the root this is the display path.
    pub name: String,
    /// Allocated bytes (`st_blocks * 512`, falling back to `st_size`).
    /// For a directory this is the sum of its children's sizes.
    pub size: u64,
    pub kind: Kind,
    pub parent: Option<NodeId>,
    /// Children sorted by size descending, then name ascending.
    pub children: Vec<NodeId>,
    /// Set when the entry could not be read (e.g. permission denied).
    pub error: Option<String>,
}

impl Node {
    pub fn new(name: impl Into<String>, kind: Kind, size: u64) -> Self {
        Node {
            name: name.into(),
            size,
            kind,
            parent: None,
            children: Vec::new(),
            error: None,
        }
    }

    pub fn is_dir(&self) -> bool {
        self.kind == Kind::Dir
    }
}

/// Arena-backed directory tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tree {
    pub nodes: Vec<Node>,
    pub root: NodeId,
}

impl Tree {
    /// Create a tree whose root is a directory named `root_name`.
    pub fn new(root_name: impl Into<String>) -> Self {
        Tree {
            nodes: vec![Node::new(root_name, Kind::Dir, 0)],
            root: 0,
        }
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id]
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Append `node` under `parent` and return its id. Does not update sizes
    /// or ordering; call [`Tree::finalize`] once the tree is complete.
    pub fn add_child(&mut self, parent: NodeId, mut node: Node) -> NodeId {
        let id = self.nodes.len();
        node.parent = Some(parent);
        self.nodes.push(node);
        self.nodes[parent].children.push(id);
        id
    }

    /// Recompute directory sizes bottom-up and sort every child list by
    /// size descending, then name ascending.
    pub fn finalize(&mut self) {
        // Children always have a larger index than their parent (they are
        // pushed after it), so a reverse pass sums sizes bottom-up.
        for id in (0..self.nodes.len()).rev() {
            if self.nodes[id].kind == Kind::Dir {
                let total: u64 = self.nodes[id]
                    .children
                    .iter()
                    .map(|&c| self.nodes[c].size)
                    .sum();
                self.nodes[id].size = total;
            }
        }
        for id in 0..self.nodes.len() {
            let mut children = std::mem::take(&mut self.nodes[id].children);
            children.sort_by(|&a, &b| {
                let (na, nb) = (&self.nodes[a], &self.nodes[b]);
                nb.size.cmp(&na.size).then_with(|| na.name.cmp(&nb.name))
            });
            self.nodes[id].children = children;
        }
    }

    /// Full path of a node, built from the root's name downwards.
    pub fn path(&self, id: NodeId) -> PathBuf {
        let mut parts = Vec::new();
        let mut cur = Some(id);
        while let Some(i) = cur {
            parts.push(self.nodes[i].name.as_str());
            cur = self.nodes[i].parent;
        }
        let mut path = PathBuf::new();
        for part in parts.into_iter().rev() {
            path.push(part);
        }
        path
    }

    /// Depth of a node (root is 0).
    pub fn depth(&self, id: NodeId) -> usize {
        let mut d = 0;
        let mut cur = self.nodes[id].parent;
        while let Some(p) = cur {
            d += 1;
            cur = self.nodes[p].parent;
        }
        d
    }

    /// Number of file (non-directory) entries and directories in the subtree rooted at `id`,
    /// excluding `id` itself.
    pub fn counts(&self, id: NodeId) -> Counts {
        let mut counts = Counts::default();
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            for &c in &self.nodes[n].children {
                match self.nodes[c].kind {
                    Kind::Dir => {
                        counts.dirs += 1;
                        stack.push(c);
                    }
                    _ => counts.files += 1,
                }
            }
        }
        counts
    }
}

/// Subtree entry counts, see [`Tree::counts`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub files: u64,
    pub dirs: u64,
}

/// Messages sent from the scan thread to the UI.
#[derive(Debug)]
pub enum ScanEvent {
    /// Periodic progress update while walking.
    Progress {
        dirs: u64,
        files: u64,
        bytes: u64,
        current: PathBuf,
    },
    /// The walk finished; the tree is finalized (sizes summed, children sorted).
    Done(Tree),
    /// The walk could not even start (e.g. the root does not exist).
    Error(String),
}

/// Format a byte count with a binary unit, e.g. `1.5 GiB`. Uses a fixed
/// width-friendly layout: one decimal below 10, none above.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// Format an integer with thousands separators, e.g. `1,234,567`.
pub fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Display name for a root path (absolute, without trailing slash unless `/`).
pub fn display_root(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.len() > 1 {
        s.trim_end_matches('/').to_string()
    } else {
        s.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Tree {
        let mut t = Tree::new("/root");
        let a = t.add_child(0, Node::new("a", Kind::Dir, 0));
        t.add_child(a, Node::new("small", Kind::File, 10));
        t.add_child(a, Node::new("big", Kind::File, 1000));
        let b = t.add_child(0, Node::new("b", Kind::Dir, 0));
        t.add_child(b, Node::new("x", Kind::File, 5));
        t.add_child(0, Node::new("top", Kind::File, 500));
        t.finalize();
        t
    }

    #[test]
    fn finalize_sums_and_sorts() {
        let t = sample();
        assert_eq!(t.node(0).size, 1515);
        let names: Vec<_> = t
            .node(0)
            .children
            .iter()
            .map(|&c| t.node(c).name.as_str())
            .collect();
        assert_eq!(names, ["a", "top", "b"]);
        let a = t.node(0).children[0];
        let a_names: Vec<_> = t
            .node(a)
            .children
            .iter()
            .map(|&c| t.node(c).name.as_str())
            .collect();
        assert_eq!(a_names, ["big", "small"]);
    }

    #[test]
    fn ties_break_by_name() {
        let mut t = Tree::new("/");
        t.add_child(0, Node::new("zeta", Kind::File, 1));
        t.add_child(0, Node::new("alpha", Kind::File, 1));
        t.finalize();
        let names: Vec<_> = t
            .node(0)
            .children
            .iter()
            .map(|&c| t.node(c).name.as_str())
            .collect();
        assert_eq!(names, ["alpha", "zeta"]);
    }

    #[test]
    fn path_and_depth() {
        let t = sample();
        let a = t.node(0).children[0];
        let big = t.node(a).children[0];
        assert_eq!(t.path(big), PathBuf::from("/root/a/big"));
        assert_eq!(t.depth(big), 2);
        assert_eq!(t.depth(0), 0);
        assert_eq!(t.path(0), PathBuf::from("/root"));
    }

    #[test]
    fn counts_exclude_self() {
        let t = sample();
        assert_eq!(t.counts(0), Counts { files: 4, dirs: 2 });
        let b = t.node(0).children[2];
        assert_eq!(t.counts(b), Counts { files: 1, dirs: 0 });
    }

    #[test]
    fn human_size_formats() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(1536), "1.5 KiB");
        assert_eq!(human_size(10 * 1024), "10 KiB");
        assert_eq!(human_size(1024 * 1024 * 1024 * 3 / 2), "1.5 GiB");
        assert_eq!(human_size(u64::MAX), "16 EiB");
    }

    #[test]
    fn group_digits_formats() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1,000");
        assert_eq!(group_digits(1234567), "1,234,567");
    }

    #[test]
    fn display_root_trims_slash() {
        assert_eq!(display_root(Path::new("/")), "/");
        assert_eq!(display_root(Path::new("/Users/x/")), "/Users/x");
    }
}
