# FileInspector

FileInspector (`fi`) is a fast, interactive disk-usage explorer for the terminal. Point it at any directory, watch the scan progress live, then navigate a size-sorted tree to find the files and folders consuming the most space.

## Run it

You need a current stable Rust toolchain.

```sh
cargo run --release -- ~/Downloads
```

With no path, `fi` inspects the current directory. To inspect an entire disk, pass its root explicitly:

```sh
fi /
```

Unreadable folders remain visible with an error marker instead of aborting the scan. Symlinks are listed but never followed, so a symlink cycle cannot trap the scanner.

## Navigate

- Move with the arrow keys or `j` / `k`; Page Up, Page Down, `g`, and `G` move farther.
- Expand or collapse a folder with Left / Right, `h` / `l`, or Space.
- Press Enter to focus on a folder and Backspace or `u` to move back out.
- Press `s` to sort by size, name, or item count.
- Mouse clicks and the scroll wheel work too.
- Press `?` for the in-app key guide and `q`, Escape, or Ctrl-C to quit.

## Install it

Install for your user with Cargo:

```sh
cargo install --path .
```

Make sure `$HOME/.cargo/bin` is on your `PATH`. For a machine-wide installation, first build a release binary and then install it in a shared binary directory:

```sh
cargo build --release
sudo install -m 0755 target/release/fi /usr/local/bin/fi
```

On macOS, inspecting protected locations may also require granting Full Disk Access to your terminal application in System Settings. FileInspector still works without it and marks locations it cannot read.

## Size accounting

FileInspector reports allocated disk usage (`st_blocks × 512` on Unix, with file length as a fallback). A directory's size is the sum of its readable descendants. This is why the number can differ from the logical byte length shown by some file browsers, particularly for sparse or compressed files.

The scan runs on a background thread so the terminal stays responsive, and results are ordered by size descending with names breaking ties consistently.
