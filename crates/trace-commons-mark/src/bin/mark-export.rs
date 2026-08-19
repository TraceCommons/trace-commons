//! Write the generated SVG documents for every packaging surface.
//!
//! Usage: `mark-export [output-dir]`, defaulting to `assets/mark` relative to
//! the current directory. CI runs it into the working tree and then
//! `git diff --exit-code assets/mark`, which is what makes the committed files
//! non-authoritative: they are there so a checkout is buildable without
//! running Rust first, not because anyone may edit them.
//!
//! Most of what this writes is SVG, which each platform's packaging turns into
//! an `.icns` or a `hicolor` entry with its own toolchain.
//!
//! The Windows tiles are the exception and are written here as PNG. They have
//! to be raster, they have to live where `Package.appxmanifest` names them, and
//! the toolchain that would otherwise produce them only exists on a machine the
//! drift check does not run on. Rendering them here is what brings them inside
//! the check. Pass `--repo-root` to write them; without it only the SVGs under
//! the output directory are written, which is what a caller exporting the
//! documents for something else wants.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut out_dir: Option<PathBuf> = None;
    let mut repo_root: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--repo-root" => match args.next() {
                Some(value) => repo_root = Some(PathBuf::from(value)),
                None => {
                    eprintln!("--repo-root needs a directory");
                    return ExitCode::FAILURE;
                }
            },
            _ if out_dir.is_none() => out_dir = Some(PathBuf::from(arg)),
            _ => {
                eprintln!("usage: mark-export [output-dir] [--repo-root <dir>]");
                return ExitCode::FAILURE;
            }
        }
    }
    let out_dir = out_dir.unwrap_or_else(|| PathBuf::from("assets/mark"));

    if let Err(err) = std::fs::create_dir_all(&out_dir) {
        eprintln!("creating {}: {err}", out_dir.display());
        return ExitCode::FAILURE;
    }

    for export in trace_commons_mark::all_exports() {
        let path = out_dir.join(export.relative_path);
        // A trailing newline so the files are ordinary text to git, diff and
        // every editor. The drift check compares bytes, so this has to be
        // written the same way every time rather than left to whatever last
        // touched the file.
        let mut contents = export.contents;
        contents.push('\n');
        if let Err(err) = std::fs::write(&path, contents) {
            eprintln!("writing {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
        println!("wrote {}", path.display());
    }

    if let Some(root) = repo_root {
        for tile in trace_commons_mark::windows_tiles() {
            let path = root.join(tile.repo_path);
            if let Some(parent) = path.parent() {
                if let Err(err) = std::fs::create_dir_all(parent) {
                    eprintln!("creating {}: {err}", parent.display());
                    return ExitCode::FAILURE;
                }
            }
            if let Err(err) = std::fs::write(&path, &tile.bytes) {
                eprintln!("writing {}: {err}", path.display());
                return ExitCode::FAILURE;
            }
            println!("wrote {}", path.display());
        }
    }

    ExitCode::SUCCESS
}
