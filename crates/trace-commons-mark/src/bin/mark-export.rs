//! Write the generated SVG documents for every packaging surface.
//!
//! Usage: `mark-export [output-dir]`, defaulting to `assets/mark` relative to
//! the current directory. CI runs it into the working tree and then
//! `git diff --exit-code assets/mark`, which is what makes the committed files
//! non-authoritative: they are there so a checkout is buildable without
//! running Rust first, not because anyone may edit them.
//!
//! SVG only. Nothing here rasterizes -- each platform's packaging turns these
//! into `.icns`, MSIX tiles or a `hicolor` entry with that platform's own
//! toolchain, on that platform's runner.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let out_dir = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("assets/mark"));
    if args.next().is_some() {
        eprintln!("usage: mark-export [output-dir]");
        return ExitCode::FAILURE;
    }

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

    ExitCode::SUCCESS
}
