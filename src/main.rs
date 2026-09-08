use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use fileinspector::{scan, tui};

#[derive(Debug, Parser)]
#[command(
    name = "fi",
    version,
    about = "Explore what is using your disk space",
    long_about = "Scan a directory, then explore its files and nested folders in an interactive terminal tree."
)]
struct Cli {
    /// Directory to inspect
    #[arg(value_name = "PATH", default_value = ".")]
    path: PathBuf,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let display_path = cli.path.display().to_string();

    let root = match cli.path.canonicalize() {
        Ok(root) => root,
        Err(error) => {
            eprintln!("fi: cannot inspect {display_path}: {error}");
            return ExitCode::from(2);
        }
    };

    if !root.is_dir() {
        eprintln!("fi: {} is not a directory", root.display());
        return ExitCode::from(2);
    }

    let events = scan::spawn(root.clone());
    match tui::run(events, root) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fi: terminal error: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_the_current_directory() {
        let cli = Cli::try_parse_from(["fi"]).unwrap();
        assert_eq!(cli.path, PathBuf::from("."));
    }

    #[test]
    fn accepts_a_root_path() {
        let cli = Cli::try_parse_from(["fi", "/tmp"]).unwrap();
        assert_eq!(cli.path, PathBuf::from("/tmp"));
    }

    #[test]
    fn rejects_more_than_one_root() {
        assert!(Cli::try_parse_from(["fi", "/tmp", "/var"]).is_err());
    }
}
