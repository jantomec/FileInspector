//! Headless scan: prints the total and the largest top-level entries.
//! Usage: cargo run --release --example dump -- <path> [top-n]
use std::path::PathBuf;
use std::time::Instant;

use fileinspector::scan;
use fileinspector::tree::{human_size, Kind};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
    let top: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(15);
    let started = Instant::now();
    let tree = match scan::scan(&path, None) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("scan failed: {e}");
            std::process::exit(1);
        }
    };
    let elapsed = started.elapsed();
    let root = tree.node(tree.root);
    let counts = tree.counts(tree.root);
    let errors = tree.nodes.iter().filter(|n| n.error.is_some()).count();
    println!(
        "{}\t{} bytes\t{}\t{} files\t{} dirs\t{} unreadable\t{:.2}s",
        tree.path(tree.root).display(),
        root.size,
        human_size(root.size),
        counts.files,
        counts.dirs,
        errors,
        elapsed.as_secs_f64()
    );
    for &c in root.children.iter().take(top) {
        let n = tree.node(c);
        let share = if root.size == 0 { 0.0 } else { n.size as f64 * 100.0 / root.size as f64 };
        let suffix = match n.kind {
            Kind::Dir => "/",
            Kind::Symlink => "@",
            _ => "",
        };
        println!(
            "{:>10}  {:5.1}%  {}{}{}",
            human_size(n.size),
            share,
            n.name,
            suffix,
            n.error.as_ref().map(|e| format!("  [{e}]")).unwrap_or_default()
        );
    }
}
