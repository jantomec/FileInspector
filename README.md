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
- Press `c` to collapse the expanded tree back to the current top level.
- Press `s` to sort by size, name, or item count.
- Mouse clicks and the scroll wheel work too.
- Press `?` for the in-app key guide and `q`, Escape, or Ctrl-C to quit.

## Install a prebuilt binary

No Rust toolchain needed. On macOS (Apple Silicon or Intel) and Linux (x86_64 or arm64):

```sh
curl -fsSL https://raw.githubusercontent.com/jantomec/FileInspector/main/install.sh | sh
```

The script downloads the matching archive from the [latest release](https://github.com/jantomec/FileInspector/releases/latest), verifies its SHA-256 checksum, and installs `fi` into `/usr/local/bin` (or `~/.local/bin` if that is not writable). Set `FI_INSTALL_DIR` to choose another directory, or `FI_VERSION=v0.1.0` to pin a version.

To install by hand, download `fi-<version>-<target>.tar.gz` from the releases page, extract it, and copy `fi` somewhere on your `PATH`. If you download with a browser on macOS, clear the quarantine flag first:

```sh
xattr -d com.apple.quarantine ./fi
```

## Install it from source

Install for your user with Cargo (from a clone, or straight from GitHub with `cargo install --git https://github.com/jantomec/FileInspector`):

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

Hard links and macOS firmlink aliases are counted once by device and inode. The shallowest path owns the allocation; other aliases stay visible as 0 B entries with a note pointing to the counted path. This keeps a full `fi /` scan from counting the macOS Data volume twice.

The scan runs on a background thread so the terminal stays responsive, and results are ordered by size descending with names breaking ties consistently.
