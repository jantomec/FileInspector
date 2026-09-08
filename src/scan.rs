use std::collections::HashMap;
use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::tree::{display_root, Kind, Node, ScanEvent, Tree};

/// Prefix used for entries that are aliases of storage already counted at a
/// preferable path. These are informational notes, not read failures.
pub(crate) const DUPLICATE_PREFIX: &str = "duplicate: ";

/// Start scanning `root` on a dedicated thread.
///
/// Dropping the receiver is enough to stop progress messages; the scan itself
/// continues until the process exits or it finishes. Keeping the filesystem
/// walk off the terminal thread ensures drawing and input remain responsive.
pub fn spawn(root: PathBuf) -> Receiver<ScanEvent> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || match scan(&root, Some(&tx)) {
        Ok(tree) => {
            let _ = tx.send(ScanEvent::Done(tree));
        }
        Err(error) => {
            let _ = tx.send(ScanEvent::Error(format!(
                "could not scan {}: {error}",
                root.display()
            )));
        }
    });
    rx
}

/// Scan a directory synchronously.
///
/// This is public primarily for non-interactive uses and tests. Interactive
/// callers should use [`spawn`] so filesystem I/O cannot stall the terminal.
pub fn scan(root: &Path, progress: Option<&Sender<ScanEvent>>) -> io::Result<Tree> {
    let root = normalize_root(root)?;
    let mut tree = Tree {
        nodes: Vec::new(),
        root: 0,
    };
    let mut state = ScanState::new(progress);

    state.emit(&root, true);
    let root_index = visit(&root, None, &mut tree, &mut state)?;
    tree.root = root_index;
    // Deduplication can move an already-scanned subtree to a shallower alias.
    // Recompute every aggregate once so all former ancestors lose its size.
    tree.finalize();
    state.emit(&root, true);

    Ok(tree)
}

fn normalize_root(root: &Path) -> io::Result<PathBuf> {
    if root.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the scan path cannot be empty",
        ));
    }

    // canonicalize also gives the CLI an immediate, useful error for a missing
    // root. Descendant symlinks are still inspected with symlink_metadata and
    // are never followed.
    root.canonicalize()
}

fn visit(
    path: &Path,
    parent: Option<usize>,
    tree: &mut Tree,
    state: &mut ScanState<'_>,
) -> io::Result<usize> {
    let metadata = fs::symlink_metadata(path)?;
    let kind = classify(&metadata);
    let is_directory = metadata.file_type().is_dir();
    let index = tree.nodes.len();

    tree.nodes.push(Node {
        name: if parent.is_none() {
            display_root(path)
        } else {
            display_name(path)
        },
        size: 0,
        kind,
        parent,
        children: Vec::new(),
        error: None,
    });

    if is_directory {
        state.dirs = state.dirs.saturating_add(1);
        if !state.register_directory(path, index, tree, &metadata) {
            visit_directory(path, index, tree, state);
        }
        state.emit(path, false);
    } else {
        let size = allocated_size(&metadata);
        tree.nodes[index].size = size;
        state.files = state.files.saturating_add(1);
        if !state.register_hard_link(path, index, tree, &metadata) {
            state.bytes = state.bytes.saturating_add(size);
        }
        state.emit(path, false);
    }

    Ok(index)
}

fn visit_directory(path: &Path, index: usize, tree: &mut Tree, state: &mut ScanState<'_>) {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) => {
            tree.nodes[index].error = Some(error.to_string());
            return;
        }
    };

    // Read the directory before recursing so OS enumeration order never leaks
    // into equal-size results or tests.
    let mut paths = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(error) => append_error(&mut tree.nodes[index].error, error),
        }
    }
    paths.sort_by(|left, right| left.as_os_str().cmp(right.as_os_str()));

    let mut children = Vec::with_capacity(paths.len());
    for child_path in paths {
        match visit(&child_path, Some(index), tree, state) {
            Ok(child) => children.push(child),
            Err(error) => {
                // Directory entries can disappear or become inaccessible during
                // a long scan. Preserve that fact on the parent and continue.
                append_error(&mut tree.nodes[index].error, error);
            }
        }
    }

    children.sort_by(|left, right| {
        tree.nodes[*right]
            .size
            .cmp(&tree.nodes[*left].size)
            .then_with(|| tree.nodes[*left].name.cmp(&tree.nodes[*right].name))
    });

    tree.nodes[index].size = children.iter().fold(0_u64, |sum, child| {
        sum.saturating_add(tree.nodes[*child].size)
    });
    tree.nodes[index].children = children;
}

fn append_error(target: &mut Option<String>, error: io::Error) {
    match target {
        Some(existing) => {
            existing.push_str("; ");
            existing.push_str(&error.to_string());
        }
        None => *target = Some(error.to_string()),
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn classify(metadata: &Metadata) -> Kind {
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        Kind::Symlink
    } else if file_type.is_dir() {
        Kind::Dir
    } else if file_type.is_file() {
        Kind::File
    } else {
        Kind::Other
    }
}

#[cfg(unix)]
fn allocated_size(metadata: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;

    // POSIX specifies st_blocks in 512-byte units. Some filesystems report
    // zero for tiny or virtual files; len() keeps those useful in the UI.
    let blocks = metadata.blocks().saturating_mul(512);
    if blocks == 0 {
        metadata.len()
    } else {
        blocks
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
fn file_identity(metadata: &Metadata) -> Option<(FileIdentity, u64)> {
    use std::os::unix::fs::MetadataExt;

    Some((
        FileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        },
        metadata.nlink(),
    ))
}

#[cfg(not(unix))]
fn file_identity(_metadata: &Metadata) -> Option<(FileIdentity, u64)> {
    None
}

/// Mark a repeated filesystem object and make the shallowest path own its
/// size and children. Returns true when `index` is a duplicate.
fn resolve_duplicate(
    seen: &mut HashMap<FileIdentity, usize>,
    identity: FileIdentity,
    path: &Path,
    index: usize,
    description: &str,
    tree: &mut Tree,
) -> bool {
    let Some(&previous) = seen.get(&identity) else {
        seen.insert(identity, index);
        return false;
    };

    let previous_path = tree.path(previous);
    let prefer_current = path.components().count() < previous_path.components().count();

    if prefer_current {
        let size = std::mem::take(&mut tree.nodes[previous].size);
        let children = std::mem::take(&mut tree.nodes[previous].children);
        let error = tree.nodes[previous].error.take();
        for child in &children {
            tree.nodes[*child].parent = Some(index);
        }
        tree.nodes[index].size = size;
        tree.nodes[index].children = children;
        tree.nodes[index].error = error;
        tree.nodes[previous].error = Some(format!(
            "{DUPLICATE_PREFIX}same {description} as {}",
            path.display()
        ));
        seen.insert(identity, index);
    } else {
        tree.nodes[index].size = 0;
        tree.nodes[index].children.clear();
        tree.nodes[index].error = Some(format!(
            "{DUPLICATE_PREFIX}same {description} as {}",
            previous_path.display()
        ));
    }

    true
}

#[cfg(not(unix))]
fn allocated_size(metadata: &Metadata) -> u64 {
    metadata.len()
}

struct ScanState<'a> {
    dirs: u64,
    files: u64,
    bytes: u64,
    progress: Option<&'a Sender<ScanEvent>>,
    last_emit: Instant,
    directories: HashMap<FileIdentity, usize>,
    hard_links: HashMap<FileIdentity, usize>,
}

impl<'a> ScanState<'a> {
    fn new(progress: Option<&'a Sender<ScanEvent>>) -> Self {
        Self {
            dirs: 0,
            files: 0,
            bytes: 0,
            progress,
            last_emit: Instant::now(),
            directories: HashMap::new(),
            hard_links: HashMap::new(),
        }
    }

    fn register_directory(
        &mut self,
        path: &Path,
        index: usize,
        tree: &mut Tree,
        metadata: &Metadata,
    ) -> bool {
        let Some((identity, _)) = file_identity(metadata) else {
            return false;
        };
        resolve_duplicate(
            &mut self.directories,
            identity,
            path,
            index,
            "directory",
            tree,
        )
    }

    fn register_hard_link(
        &mut self,
        path: &Path,
        index: usize,
        tree: &mut Tree,
        metadata: &Metadata,
    ) -> bool {
        let Some((identity, links)) = file_identity(metadata) else {
            return false;
        };
        if links <= 1 {
            return false;
        }
        resolve_duplicate(&mut self.hard_links, identity, path, index, "file", tree)
    }

    fn emit(&mut self, current: &Path, force: bool) {
        if !force && self.last_emit.elapsed() < Duration::from_millis(75) {
            return;
        }
        let Some(progress) = self.progress else {
            return;
        };

        let _ = progress.send(ScanEvent::Progress {
            dirs: self.dirs,
            files: self.files,
            bytes: self.bytes,
            current: current.to_path_buf(),
        });
        self.last_emit = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let suffix = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "fileinspector-test-{}-{suffix}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, relative: &str, bytes: usize) {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            let mut file = File::create(path).unwrap();
            file.write_all(&vec![b'x'; bytes]).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn aggregates_sizes_and_sorts_largest_first() {
        let temp = TempDir::new();
        temp.write("small.txt", 1);
        temp.write("nested/largest.bin", 16_000);

        let tree = scan(&temp.0, None).unwrap();
        let root = &tree.nodes[tree.root];
        let first = &tree.nodes[root.children[0]];
        let second = &tree.nodes[root.children[1]];

        assert_eq!(root.size, first.size + second.size);
        assert_eq!(first.name, "nested");
        assert_eq!(second.name, "small.txt");
        assert!(first.size >= second.size);
    }

    #[test]
    fn equal_sized_children_are_sorted_by_name() {
        let temp = TempDir::new();
        temp.write("z.txt", 8);
        temp.write("a.txt", 8);

        let tree = scan(&temp.0, None).unwrap();
        let names: Vec<&str> = tree.nodes[tree.root]
            .children
            .iter()
            .map(|child| tree.nodes[*child].name.as_str())
            .collect();

        assert_eq!(names, ["a.txt", "z.txt"]);
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_directory_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new();
        temp.write("directory/file.txt", 32);
        symlink(&temp.0, temp.0.join("directory/back-to-root")).unwrap();

        let tree = scan(&temp.0, None).unwrap();
        let symlink_nodes: Vec<&Node> = tree
            .nodes
            .iter()
            .filter(|node| matches!(node.kind, Kind::Symlink))
            .collect();

        assert_eq!(symlink_nodes.len(), 1);
        assert!(symlink_nodes[0].children.is_empty());
        assert!(tree.nodes.len() < 10, "a symlink loop was followed");
    }

    #[cfg(unix)]
    #[test]
    fn counts_hard_link_storage_once() {
        let temp = TempDir::new();
        temp.write("original.bin", 16_000);
        fs::hard_link(temp.0.join("original.bin"), temp.0.join("second-name.bin")).unwrap();
        let expected = allocated_size(&fs::metadata(temp.0.join("original.bin")).unwrap());

        let tree = scan(&temp.0, None).unwrap();
        let root = tree.node(tree.root);
        let duplicate = tree
            .nodes
            .iter()
            .find(|node| {
                node.error
                    .as_deref()
                    .is_some_and(|error| error.starts_with(DUPLICATE_PREFIX))
            })
            .expect("one hard link should be marked as a duplicate");

        assert_eq!(root.size, expected);
        assert_eq!(duplicate.size, 0);
        assert!(duplicate.children.is_empty());
    }

    #[test]
    fn repeated_directory_is_owned_by_the_shallowest_path() {
        let mut tree = Tree::new("/");
        let system = tree.add_child(0, Node::new("System", Kind::Dir, 0));
        let volumes = tree.add_child(system, Node::new("Volumes", Kind::Dir, 0));
        let data = tree.add_child(volumes, Node::new("Data", Kind::Dir, 0));
        let deep_users = tree.add_child(data, Node::new("Users", Kind::Dir, 0));
        let file = tree.add_child(deep_users, Node::new("large.bin", Kind::File, 4096));
        tree.finalize();

        let identity = FileIdentity {
            device: 1,
            inode: 42,
        };
        let mut seen = HashMap::new();
        assert!(!resolve_duplicate(
            &mut seen,
            identity,
            Path::new("/System/Volumes/Data/Users"),
            deep_users,
            "directory",
            &mut tree,
        ));

        let shallow_users = tree.add_child(0, Node::new("Users", Kind::Dir, 0));
        assert!(resolve_duplicate(
            &mut seen,
            identity,
            Path::new("/Users"),
            shallow_users,
            "directory",
            &mut tree,
        ));
        tree.finalize();

        assert_eq!(tree.node(shallow_users).size, 4096);
        assert_eq!(tree.node(shallow_users).children, [file]);
        assert_eq!(tree.node(file).parent, Some(shallow_users));
        assert_eq!(tree.node(deep_users).size, 0);
        assert!(tree.node(deep_users).children.is_empty());
        assert!(tree.node(deep_users).error.as_deref().is_some_and(|error| {
            error == format!("{DUPLICATE_PREFIX}same directory as /Users")
        }));
        assert_eq!(tree.node(tree.root).size, 4096);
    }

    #[cfg(unix)]
    #[test]
    fn keeps_unreadable_directories_as_error_nodes() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new();
        let denied = temp.0.join("denied");
        fs::create_dir(&denied).unwrap();
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o000)).unwrap();
        let permissions_are_enforced = fs::read_dir(&denied).is_err();

        let result = scan(&temp.0, None);
        // Restore access before assertions so TempDir can always clean up.
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o700)).unwrap();
        let tree = result.unwrap();

        // Root-run CI can still read mode-000 directories, so it cannot exercise
        // this OS error path. Normal user runs must retain the failed node.
        if permissions_are_enforced {
            let denied_node = tree
                .nodes
                .iter()
                .find(|node| node.name == "denied")
                .expect("the unreadable directory should remain visible");
            assert!(denied_node.error.is_some());
            assert!(denied_node.children.is_empty());
        }
    }

    #[test]
    fn emits_initial_progress_and_done_from_background_thread() {
        let temp = TempDir::new();
        temp.write("file.txt", 64);

        let events = spawn(temp.0.clone());
        let first = events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(first, ScanEvent::Progress { .. }));

        let done = events
            .into_iter()
            .find(|event| matches!(event, ScanEvent::Done(_)))
            .expect("scanner should emit a completed tree");
        assert!(matches!(done, ScanEvent::Done(_)));
    }

    #[test]
    fn missing_root_is_an_error() {
        let temp = TempDir::new();
        let missing = temp.0.join("not-here");
        let error = scan(&missing, None).expect_err("missing root must fail");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
