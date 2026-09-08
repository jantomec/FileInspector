//! UI state and input handling, kept free of terminal I/O so it can be unit tested.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui_core::layout::Rect;

use crate::tree::{Counts, Kind, NodeId, ScanEvent, Tree};

/// Ordering of siblings in the tree view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    /// Largest first (ties by name). The default.
    Size,
    /// Case-insensitive name, directories and files mixed.
    Name,
    /// Most entries (files + directories) first.
    Items,
}

impl SortMode {
    pub fn next(self) -> Self {
        match self {
            SortMode::Size => SortMode::Name,
            SortMode::Name => SortMode::Items,
            SortMode::Items => SortMode::Size,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortMode::Size => "size",
            SortMode::Name => "name",
            SortMode::Items => "items",
        }
    }
}

/// Latest scan progress figures.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    pub dirs: u64,
    pub files: u64,
    pub bytes: u64,
    pub current: PathBuf,
}

/// Where the application is in its lifecycle.
#[derive(Debug)]
pub enum Phase {
    Scanning(Progress),
    Ready(Tree),
    Failed(String),
}

/// One visible line of the tree view.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: NodeId,
    /// 0 for direct children of the view root.
    pub depth: usize,
    /// For each ancestor level between the view root and this row: whether
    /// that ancestor was the last of its siblings (so no guide line continues).
    pub guides: Vec<bool>,
    /// Last among its siblings.
    pub last: bool,
    pub expanded: bool,
    /// Fraction of the parent's size, 0..=1.
    pub share: f64,
}

/// Complete UI state.
pub struct App {
    pub root: PathBuf,
    pub phase: Phase,
    pub started: Instant,
    pub finished: Option<Duration>,
    /// Directory whose children fill the list ("zoomed" directory).
    pub view_root: NodeId,
    pub expanded: HashSet<NodeId>,
    pub rows: Vec<Row>,
    pub selected: usize,
    /// Index of the first row drawn.
    pub offset: usize,
    /// Rows that fit on screen at the last draw.
    pub page: usize,
    /// Screen area the rows were drawn into at the last draw (for mouse hits).
    pub rows_area: Rect,
    pub sort: SortMode,
    pub show_help: bool,
    pub quit: bool,
    /// Entries the scanner could not read.
    pub unreadable: u64,
    pub tick: u64,
    counts: HashMap<NodeId, Counts>,
}

impl App {
    pub fn new(root: PathBuf) -> Self {
        App {
            root,
            phase: Phase::Scanning(Progress::default()),
            started: Instant::now(),
            finished: None,
            view_root: 0,
            expanded: HashSet::new(),
            rows: Vec::new(),
            selected: 0,
            offset: 0,
            page: 1,
            rows_area: Rect::default(),
            sort: SortMode::Size,
            show_help: false,
            quit: false,
            unreadable: 0,
            tick: 0,
            counts: HashMap::new(),
        }
    }

    /// Build an app directly from a finished tree (tests and non-interactive callers).
    pub fn with_tree(root: PathBuf, tree: Tree) -> Self {
        let mut app = App::new(root);
        app.apply(ScanEvent::Done(tree));
        app
    }

    /// Feed one scanner message into the state.
    pub fn apply(&mut self, event: ScanEvent) {
        match event {
            ScanEvent::Progress {
                dirs,
                files,
                bytes,
                current,
            } => {
                if let Phase::Scanning(p) = &mut self.phase {
                    *p = Progress {
                        dirs,
                        files,
                        bytes,
                        current,
                    };
                }
            }
            ScanEvent::Done(tree) => {
                if self.finished.is_none() {
                    self.finished = Some(self.started.elapsed());
                }
                self.unreadable = tree
                    .nodes
                    .iter()
                    .filter(|n| n.read_error().is_some())
                    .count() as u64;
                self.view_root = tree.root;
                self.expanded.clear();
                self.expanded.insert(tree.root);
                self.counts.clear();
                self.phase = Phase::Ready(tree);
                self.selected = 0;
                self.offset = 0;
                self.rebuild();
            }
            ScanEvent::Error(message) => {
                if self.finished.is_none() {
                    self.finished = Some(self.started.elapsed());
                }
                self.phase = Phase::Failed(message);
            }
        }
    }

    pub fn tree(&self) -> Option<&Tree> {
        match &self.phase {
            Phase::Ready(t) => Some(t),
            _ => None,
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.phase, Phase::Ready(_))
    }

    /// Time spent scanning (frozen once the scan ended).
    pub fn elapsed(&self) -> Duration {
        self.finished.unwrap_or_else(|| self.started.elapsed())
    }

    /// Cached subtree entry counts for a node.
    pub fn counts_of(&mut self, id: NodeId) -> Counts {
        let Phase::Ready(tree) = &self.phase else {
            return Counts::default();
        };
        *self.counts.entry(id).or_insert_with(|| tree.counts(id))
    }

    pub fn selected_row(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    /// Recompute the visible rows, keeping the selection on the same node when possible.
    fn rebuild(&mut self) {
        let keep = self.rows.get(self.selected).map(|r| r.id);
        let Phase::Ready(tree) = &self.phase else {
            self.rows.clear();
            return;
        };
        self.rows = build_rows(
            tree,
            self.view_root,
            &self.expanded,
            self.sort,
            &mut self.counts,
        );
        self.selected = keep
            .and_then(|id| self.rows.iter().position(|r| r.id == id))
            .unwrap_or(self.selected)
            .min(self.rows.len().saturating_sub(1));
    }

    // ----- navigation ---------------------------------------------------

    pub fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() {
            self.selected = 0;
            return;
        }
        let max = self.rows.len() as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, max) as usize;
    }

    pub fn page_down(&mut self) {
        self.move_by(self.page.max(1) as isize);
    }

    pub fn page_up(&mut self) {
        self.move_by(-(self.page.max(1) as isize));
    }

    pub fn home(&mut self) {
        self.selected = 0;
    }

    pub fn end(&mut self) {
        self.selected = self.rows.len().saturating_sub(1);
    }

    fn selected_dir(&self) -> Option<NodeId> {
        let row = self.rows.get(self.selected)?;
        let tree = self.tree()?;
        tree.node(row.id).is_dir().then_some(row.id)
    }

    pub fn expand_selected(&mut self) {
        if let Some(id) = self.selected_dir() {
            if self.expanded.insert(id) {
                self.rebuild();
            }
        }
    }

    pub fn collapse_selected(&mut self) -> bool {
        if let Some(id) = self.selected_dir() {
            if self.expanded.remove(&id) {
                self.rebuild();
                return true;
            }
        }
        false
    }

    pub fn toggle_selected(&mut self) {
        if let Some(row) = self.rows.get(self.selected) {
            if row.expanded {
                self.collapse_selected();
            } else {
                self.expand_selected();
            }
        }
    }

    /// Right: expand a collapsed directory, or step into an expanded one.
    pub fn go_right(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        if row.expanded {
            let has_child = self
                .rows
                .get(self.selected + 1)
                .is_some_and(|next| next.depth > row.depth);
            if has_child {
                self.selected += 1;
            }
        } else {
            self.expand_selected();
        }
    }

    /// Left: collapse an expanded directory, else jump to the parent row,
    /// else (already at the top level) zoom out.
    pub fn go_left(&mut self) {
        if self.collapse_selected() {
            return;
        }
        let Some(row) = self.rows.get(self.selected) else {
            self.zoom_out();
            return;
        };
        if row.depth == 0 {
            self.zoom_out();
            return;
        }
        let Some(parent) = self.tree().and_then(|t| t.node(row.id).parent) else {
            return;
        };
        if let Some(idx) = self.rows.iter().position(|r| r.id == parent) {
            self.selected = idx;
        }
    }

    /// Make the selected directory the view root.
    pub fn zoom_in(&mut self) {
        if let Some(id) = self.selected_dir() {
            self.view_root = id;
            self.expanded.insert(id);
            self.rows.clear();
            self.selected = 0;
            self.offset = 0;
            self.rebuild();
        }
    }

    /// Move the view root to its parent, selecting the directory we came from.
    pub fn zoom_out(&mut self) {
        let old = self.view_root;
        let Some(parent) = self.tree().and_then(|t| t.node(old).parent) else {
            return;
        };
        self.view_root = parent;
        self.expanded.insert(parent);
        self.rows.clear();
        self.rebuild();
        self.selected = self.rows.iter().position(|r| r.id == old).unwrap_or(0);
    }

    pub fn cycle_sort(&mut self) {
        self.sort = self.sort.next();
        self.rebuild();
    }

    /// Collapse every directory below the view root, keeping the selection on
    /// the top-level ancestor of the current row.
    pub fn collapse_all(&mut self) {
        let view_root = self.view_root;
        let keep = self.rows.get(self.selected).and_then(|row| {
            let tree = self.tree()?;
            let mut id = row.id;
            while let Some(parent) = tree.node(id).parent {
                if parent == view_root {
                    break;
                }
                id = parent;
            }
            Some(id)
        });
        self.expanded.retain(|&id| id == view_root);
        self.rows.clear();
        self.rebuild();
        if let Some(id) = keep {
            self.selected = self.rows.iter().position(|r| r.id == id).unwrap_or(0);
        }
    }

    /// Keep the selection inside the visible window of `height` rows.
    pub fn ensure_visible(&mut self, height: usize) {
        let height = height.max(1);
        self.page = height;
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + height {
            self.offset = self.selected + 1 - height;
        }
        let max_offset = self.rows.len().saturating_sub(height);
        self.offset = self.offset.min(max_offset);
    }

    // ----- input ----------------------------------------------------------

    pub fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C')) {
            self.quit = true;
            return;
        }
        if self.show_help {
            self.show_help = false;
            return;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('?') => self.show_help = true,
            _ if !self.is_ready() => {}
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::PageUp => self.page_up(),
            KeyCode::PageDown => self.page_down(),
            KeyCode::Char('u') if ctrl => self.page_up(),
            KeyCode::Char('d') if ctrl => self.page_down(),
            KeyCode::Home | KeyCode::Char('g') => self.home(),
            KeyCode::End | KeyCode::Char('G') => self.end(),
            KeyCode::Right | KeyCode::Char('l') => self.go_right(),
            KeyCode::Left | KeyCode::Char('h') => self.go_left(),
            KeyCode::Char(' ') => self.toggle_selected(),
            KeyCode::Enter => self.zoom_in(),
            KeyCode::Backspace | KeyCode::Char('u') => self.zoom_out(),
            KeyCode::Char('s') => self.cycle_sort(),
            KeyCode::Char('c') => self.collapse_all(),
            _ => {}
        }
    }

    pub fn on_mouse(&mut self, mouse: MouseEvent) {
        if !self.is_ready() || self.show_help {
            return;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp => self.move_by(-3),
            MouseEventKind::ScrollDown => self.move_by(3),
            MouseEventKind::Down(_) => {
                let area = self.rows_area;
                let inside = mouse.column >= area.x
                    && mouse.column < area.x + area.width
                    && mouse.row >= area.y
                    && mouse.row < area.y + area.height;
                if inside {
                    let idx = self.offset + (mouse.row - area.y) as usize;
                    if idx < self.rows.len() {
                        if idx == self.selected {
                            self.toggle_selected();
                        } else {
                            self.selected = idx;
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Children of `id` in the order requested by `sort`.
fn sorted_children(
    tree: &Tree,
    id: NodeId,
    sort: SortMode,
    counts: &mut HashMap<NodeId, Counts>,
) -> Vec<NodeId> {
    let mut kids = tree.node(id).children.clone();
    match sort {
        SortMode::Size => kids.sort_by(|&a, &b| {
            let (na, nb) = (tree.node(a), tree.node(b));
            nb.size.cmp(&na.size).then_with(|| na.name.cmp(&nb.name))
        }),
        SortMode::Name => kids.sort_by(|&a, &b| {
            let (na, nb) = (tree.node(a), tree.node(b));
            na.name
                .to_lowercase()
                .cmp(&nb.name.to_lowercase())
                .then_with(|| na.name.cmp(&nb.name))
        }),
        SortMode::Items => {
            let total = |c: NodeId, counts: &mut HashMap<NodeId, Counts>| {
                let n = *counts.entry(c).or_insert_with(|| tree.counts(c));
                n.files + n.dirs
            };
            let keyed: Vec<(u64, NodeId)> = kids.iter().map(|&c| (total(c, counts), c)).collect();
            let mut keyed = keyed;
            keyed.sort_by(|a, b| {
                let (na, nb) = (tree.node(a.1), tree.node(b.1));
                b.0.cmp(&a.0)
                    .then_with(|| nb.size.cmp(&na.size))
                    .then_with(|| na.name.cmp(&nb.name))
            });
            kids = keyed.into_iter().map(|(_, c)| c).collect();
        }
    }
    kids
}

/// Flatten the expanded part of the tree under `view_root` into rows.
fn build_rows(
    tree: &Tree,
    view_root: NodeId,
    expanded: &HashSet<NodeId>,
    sort: SortMode,
    counts: &mut HashMap<NodeId, Counts>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut guides = Vec::new();
    push_children(
        tree,
        view_root,
        0,
        expanded,
        sort,
        counts,
        &mut guides,
        &mut rows,
    );
    rows
}

#[allow(clippy::too_many_arguments)]
fn push_children(
    tree: &Tree,
    id: NodeId,
    depth: usize,
    expanded: &HashSet<NodeId>,
    sort: SortMode,
    counts: &mut HashMap<NodeId, Counts>,
    guides: &mut Vec<bool>,
    rows: &mut Vec<Row>,
) {
    let kids = sorted_children(tree, id, sort, counts);
    let parent_size = tree.node(id).size;
    let n = kids.len();
    for (i, child) in kids.into_iter().enumerate() {
        let node = tree.node(child);
        let last = i + 1 == n;
        let is_expanded = node.kind == Kind::Dir && expanded.contains(&child);
        let share = if parent_size == 0 {
            0.0
        } else {
            (node.size as f64 / parent_size as f64).clamp(0.0, 1.0)
        };
        rows.push(Row {
            id: child,
            depth,
            guides: guides.clone(),
            last,
            expanded: is_expanded,
            share,
        });
        if is_expanded {
            guides.push(last);
            push_children(tree, child, depth + 1, expanded, sort, counts, guides, rows);
            guides.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::Node;

    /// /root
    ///   a/  (1010)  -> big 1000, small 10
    ///   top (500)
    ///   b/  (5)     -> x 5
    ///   empty/ (0)
    fn sample() -> Tree {
        let mut t = Tree::new("/root");
        let a = t.add_child(0, Node::new("a", Kind::Dir, 0));
        t.add_child(a, Node::new("small", Kind::File, 10));
        t.add_child(a, Node::new("big", Kind::File, 1000));
        let b = t.add_child(0, Node::new("b", Kind::Dir, 0));
        t.add_child(b, Node::new("x", Kind::File, 5));
        t.add_child(0, Node::new("top", Kind::File, 500));
        t.add_child(0, Node::new("empty", Kind::Dir, 0));
        t.finalize();
        t
    }

    fn app() -> App {
        App::with_tree(PathBuf::from("/root"), sample())
    }

    fn names(app: &App) -> Vec<String> {
        let tree = app.tree().unwrap();
        app.rows
            .iter()
            .map(|r| format!("{}{}", "  ".repeat(r.depth), tree.node(r.id).name))
            .collect()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn done_shows_root_children_sorted_by_size() {
        let app = app();
        assert!(app.is_ready());
        assert_eq!(names(&app), ["a", "top", "b", "empty"]);
        assert_eq!(app.selected, 0);
        assert!((app.rows[0].share - 1010.0 / 1515.0).abs() < 1e-9);
        assert!(app.rows[3].last);
        assert!(!app.rows[0].last);
    }

    #[test]
    fn expand_and_collapse_nest_rows() {
        let mut app = app();
        app.on_key(key(KeyCode::Right));
        assert_eq!(names(&app), ["a", "  big", "  small", "top", "b", "empty"]);
        assert!(app.rows[0].expanded);
        assert_eq!(app.rows[1].guides, vec![false]);
        assert!((app.rows[1].share - 1000.0 / 1010.0).abs() < 1e-9);
        // Right on an expanded dir steps into its first child.
        app.on_key(key(KeyCode::Right));
        assert_eq!(app.selected, 1);
        // Left from a child jumps to the parent row.
        app.on_key(key(KeyCode::Left));
        assert_eq!(app.selected, 0);
        // Left on an expanded dir collapses it.
        app.on_key(key(KeyCode::Left));
        assert_eq!(names(&app), ["a", "top", "b", "empty"]);
        assert!(!app.rows[0].expanded);
    }

    #[test]
    fn expanding_a_file_does_nothing() {
        let mut app = app();
        app.move_by(1);
        app.on_key(key(KeyCode::Char(' ')));
        assert_eq!(names(&app), ["a", "top", "b", "empty"]);
    }

    #[test]
    fn selection_survives_rebuild() {
        let mut app = app();
        app.move_by(2); // "b"
        app.home();
        app.on_key(key(KeyCode::Char(' '))); // expand "a" while selected 0
        app.selected = 4; // "b"
        app.cycle_sort(); // Name: a, b, empty, top (a expanded: big, small)
        assert_eq!(names(&app), ["a", "  big", "  small", "b", "empty", "top"]);
        assert_eq!(
            app.tree().unwrap().node(app.rows[app.selected].id).name,
            "b"
        );
    }

    #[test]
    fn zoom_in_and_out() {
        let mut app = app();
        app.on_key(key(KeyCode::Enter)); // zoom into "a"
        assert_eq!(app.tree().unwrap().node(app.view_root).name, "a");
        assert_eq!(names(&app), ["big", "small"]);
        assert_eq!(app.selected, 0);
        app.on_key(key(KeyCode::Backspace));
        assert_eq!(app.view_root, 0);
        assert_eq!(
            app.tree().unwrap().node(app.rows[app.selected].id).name,
            "a"
        );
        // Left at top level zooms out; at the root it is a no-op.
        app.on_key(key(KeyCode::Left));
        assert_eq!(app.view_root, 0);
    }

    #[test]
    fn enter_on_file_is_noop() {
        let mut app = app();
        app.move_by(1);
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.view_root, 0);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn zoom_into_empty_dir_has_no_rows() {
        let mut app = app();
        app.end();
        app.on_key(key(KeyCode::Enter));
        assert!(app.rows.is_empty());
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.selected, 0);
        app.on_key(key(KeyCode::Left)); // zoom back out
        assert_eq!(app.view_root, 0);
        assert_eq!(
            app.tree().unwrap().node(app.rows[app.selected].id).name,
            "empty"
        );
    }

    #[test]
    fn collapse_all_keeps_top_level_ancestor() {
        let mut app = app();
        app.expand_selected(); // a
        app.selected = 2; // small
        app.on_key(key(KeyCode::Char('c')));
        assert_eq!(names(&app), ["a", "top", "b", "empty"]);
        assert_eq!(app.selected, 0);
        assert!(app.expanded.contains(&app.view_root));
        assert_eq!(app.expanded.len(), 1);
    }

    #[test]
    fn sort_modes_cycle() {
        let mut app = app();
        app.cycle_sort();
        assert_eq!(app.sort, SortMode::Name);
        assert_eq!(names(&app), ["a", "b", "empty", "top"]);
        app.cycle_sort();
        assert_eq!(app.sort, SortMode::Items);
        assert_eq!(names(&app), ["a", "b", "top", "empty"]);
        app.cycle_sort();
        assert_eq!(app.sort, SortMode::Size);
    }

    #[test]
    fn scrolling_keeps_selection_visible() {
        let mut app = app();
        app.ensure_visible(2);
        assert_eq!(app.offset, 0);
        app.end();
        app.ensure_visible(2);
        assert_eq!(app.offset, 2);
        app.home();
        app.ensure_visible(2);
        assert_eq!(app.offset, 0);
        app.page_down();
        assert_eq!(app.selected, 2);
        app.page_up();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn progress_and_failure_phases() {
        let mut app = App::new(PathBuf::from("/x"));
        assert!(!app.is_ready());
        app.apply(ScanEvent::Progress {
            dirs: 1,
            files: 2,
            bytes: 3,
            current: PathBuf::from("/x/y"),
        });
        match &app.phase {
            Phase::Scanning(p) => assert_eq!(p.files, 2),
            _ => panic!("expected scanning"),
        }
        // Navigation keys are ignored while scanning; quit still works.
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.selected, 0);
        app.apply(ScanEvent::Error("boom".into()));
        assert!(matches!(&app.phase, Phase::Failed(m) if m == "boom"));
        app.on_key(key(KeyCode::Char('q')));
        assert!(app.quit);
    }

    #[test]
    fn help_toggle_and_ctrl_c() {
        let mut app = app();
        app.on_key(key(KeyCode::Char('?')));
        assert!(app.show_help);
        app.on_key(key(KeyCode::Down)); // any key closes help and is consumed
        assert!(!app.show_help);
        assert_eq!(app.selected, 0);
        app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.quit);
    }

    #[test]
    fn unreadable_entries_are_counted() {
        let mut t = sample();
        t.node_mut(1).error = Some("permission denied".into());
        t.node_mut(2).error = Some(format!(
            "{}same file as /root/a/big",
            crate::tree::DUPLICATE_PREFIX
        ));
        let app = App::with_tree(PathBuf::from("/root"), t);
        assert_eq!(app.unreadable, 1);
    }

    #[test]
    fn mouse_click_selects_and_toggles() {
        let mut app = app();
        app.rows_area = Rect::new(2, 3, 40, 10);
        let click = |row: u16| MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 5,
            row,
            modifiers: KeyModifiers::NONE,
        };
        app.on_mouse(click(5)); // row index 2 -> "b"
        assert_eq!(app.selected, 2);
        app.on_mouse(click(5)); // same row again toggles expansion
        assert_eq!(names(&app), ["a", "top", "b", "  x", "empty"]);
        app.on_mouse(click(30)); // outside the list: ignored
        assert_eq!(app.selected, 2);
    }
}
