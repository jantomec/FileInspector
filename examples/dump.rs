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
    let errors = tree
        .nodes
        .iter()
        .filter(|n| n.read_error().is_some())
        .count();
    let duplicates = tree
        .nodes
        .iter()
        .filter(|n| n.duplicate_note().is_some())
        .count();
    println!(
        "{}\t{} bytes\t{}\t{} files\t{} dirs\t{} unreadable\t{} duplicates\t{:.2}s",
        tree.path(tree.root).display(),
        root.size,
        human_size(root.size),
        counts.files,
        counts.dirs,
        errors,
        duplicates,
        elapsed.as_secs_f64()
    );
    // Where do read errors cluster? Most common messages, then shallowest paths.
    let mut histogram: std::collections::HashMap<String, usize> = Default::default();
    let mut error_ids: Vec<usize> = Vec::new();
    for (id, n) in tree.nodes.iter().enumerate() {
        if let Some(e) = n.read_error() {
            let key: String = e.chars().take(70).collect();
            *histogram.entry(key).or_default() += 1;
            error_ids.push(id);
        }
    }
    let mut histogram: Vec<_> = histogram.into_iter().collect();
    histogram.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    for (msg, count) in histogram.iter().take(8) {
        println!("errors {count:>8}  {msg}");
    }
    error_ids.sort_by_key(|&id| tree.depth(id));
    for &id in error_ids.iter().take(12) {
        println!("error at {}", tree.path(id).display());
    }
    for &c in root.children.iter().take(top) {
        let n = tree.node(c);
        let share = if root.size == 0 {
            0.0
        } else {
            n.size as f64 * 100.0 / root.size as f64
        };
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
            n.error
                .as_ref()
                .map(|e| format!("  [{e}]"))
                .unwrap_or_default()
        );
    }
}
