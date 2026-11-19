//! The argv check on Windows (W14-D1 (c)): every terminal offered on
//! Windows must take a folder and the helper's command as argv elements
//! that arrive intact. Windows Terminal is the one offered today; any
//! terminal added to [`TerminalApp::WINDOWS`] is started by
//! `real_windows_terminals_receive_the_folder_and_every_argument` too, so
//! it cannot join without passing it.
//!
//! On Windows argv does not travel as a vector. `std::process::Command`
//! writes one command line, the terminal splits it again, and Windows
//! Terminal writes a second command line for the program it starts, which
//! splits it once more. The unit tests below pin the vector `plan` hands
//! the OS; only a real launch shows what reaches the program, so that is
//! the ignored test, run on a Windows desktop by
//! `windows/scripts/managed-terminal-launch-smoke.ps1`.
//!
//! The expected argv is written out literally per terminal, never built
//! from `terminal_args` (the function under test).
use super::*;
use std::ffi::OsString;

/// Windows folders a launch must pass through unchanged: spaces, a single
/// quote, the characters `cmd.exe` treats specially, `%` around a variable
/// name, non-ASCII text outside the Basic Multilingual Plane, a leading
/// dash, a backtick and `$`, and a drive root (a trailing backslash).
/// A double quote cannot occur in a Windows file name.
fn hostile_folders() -> Vec<PathBuf> {
    [
        r"C:\Users\x\My Projects\app",
        r"C:\Users\x\it's 'single' q",
        r"C:\Users\x\a & b ^ c (d) !e! =f, g",
        r"C:\Users\x\100% %USERPROFILE% x",
        "C:\\Users\\x\\proyecto-a\u{f1}o\\\u{65e5}\u{672c}\u{8a9e} \u{1f642}",
        r"C:\Users\x\-d",
        r"C:\Users\x\back`tick $HOME",
        r"C:\",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}

const WT: &str = r"C:\Users\x\AppData\Local\Microsoft\WindowsApps\wt.exe";
const HELPER: &str = r"C:\Program Files\NEAR AI\near-ai.exe";
const CONFIG: &str = r"C:\Users\x\AppData\Roaming\near ai";

fn wt_plan(cwd: &Path) -> Result<Plan, ManagedError> {
    plan(
        TerminalApp::WindowsTerminal,
        Path::new(WT),
        Path::new(HELPER),
        Path::new(CONFIG),
        cwd,
        Uuid::nil(),
        "abcdef01",
    )
}

fn strings(args: &[OsString]) -> Vec<&str> {
    args.iter().map(|a| a.to_str().unwrap()).collect()
}

#[test]
fn windows_terminal_gets_the_folder_and_the_helper_as_argv() {
    let plan = wt_plan(Path::new(r"C:\Users\x\My Projects\app")).unwrap();
    assert_eq!(plan.program, Path::new(WT));
    assert_eq!(plan.cwd, None);
    assert_eq!(
        strings(&plan.args),
        [
            "new-tab",
            "--title",
            "NEAR AI",
            "-d",
            r"C:\Users\x\My Projects\app",
            r"C:\Program Files\NEAR AI\near-ai.exe",
            "--config-dir",
            r"C:\Users\x\AppData\Roaming\near ai",
            "redeem-launch",
            "--session-id",
            "00000000-0000-0000-0000-000000000000",
            "--ticket",
            "abcdef01",
        ]
    );
}

#[test]
fn a_hostile_windows_folder_is_one_verbatim_argument() {
    for cwd in hostile_folders() {
        let plan = wt_plan(&cwd).unwrap_or_else(|e| panic!("{cwd:?}: {e:?}"));
        let args = strings(&plan.args);
        // The folder is the `-d` value, whole, and nowhere else.
        assert_eq!(
            &args[..4],
            ["new-tab", "--title", "NEAR AI", "-d"],
            "{cwd:?}"
        );
        assert_eq!(Path::new(args[4]), cwd, "{cwd:?}");
        assert_eq!(args[5], HELPER, "{cwd:?}");
        assert_eq!(args.len(), 13, "{cwd:?}: {args:?}");
    }
}

/// A `;` anywhere in Windows Terminal's command line starts another
/// command, so a folder, helper or config folder holding one is refused,
/// never escaped.
#[test]
fn a_semicolon_is_refused_in_every_windows_terminal_argument() {
    assert_eq!(
        wt_plan(Path::new(r"C:\Users\x\a;b")),
        Err(ManagedError::InvalidRequest)
    );
    let with = |helper: &str, config: &str| {
        plan(
            TerminalApp::WindowsTerminal,
            Path::new(WT),
            Path::new(helper),
            Path::new(config),
            Path::new(r"C:\Users\x"),
            Uuid::nil(),
            "abcdef01",
        )
    };
    assert_eq!(
        with(r"C:\a;b\near-ai.exe", CONFIG),
        Err(ManagedError::InvalidRequest)
    );
    assert_eq!(
        with(HELPER, r"C:\Users\x\c;fg"),
        Err(ManagedError::InvalidRequest)
    );
}

/// Windows Terminal expands `%NAME%` in the command it starts (measured on
/// Windows Server 2025 with Windows Terminal 1.25.3131.0, 2026-11-19: a
/// config folder `cfg %USERNAME% x` reached the helper as `cfg tcdev x`),
/// and has no escape for it. A helper path or config folder holding `%` is
/// refused. The folder, told by `-d`, arrived unexpanded
/// (`100% %USERPROFILE% x`), so it is not refused.
#[test]
fn a_percent_in_the_command_is_refused_for_windows_terminal() {
    let with = |helper: &str, config: &str| {
        plan(
            TerminalApp::WindowsTerminal,
            Path::new(WT),
            Path::new(helper),
            Path::new(config),
            Path::new(r"C:\Users\x\100% %USERPROFILE% x"),
            Uuid::nil(),
            "abcdef01",
        )
    };
    assert_eq!(
        with(HELPER, r"C:\Users\x\cfg %USERNAME% x"),
        Err(ManagedError::InvalidRequest)
    );
    assert_eq!(
        with(r"C:\Program Files\100%\near-ai.exe", CONFIG),
        Err(ManagedError::InvalidRequest)
    );
    assert!(with(HELPER, CONFIG).is_ok());
}

/// A drive-relative folder (`C:proj`, relative to that drive's current
/// folder) and a rooted one with no drive (`\proj`, on whichever drive is
/// current) are not absolute on Windows, and are refused: a launch never
/// lets the terminal's own current folder decide where it opens (L4).
#[test]
fn a_drive_relative_or_driveless_folder_is_refused() {
    for cwd in [r"C:proj", r"\proj", r"proj\sub"] {
        assert_eq!(
            wt_plan(Path::new(cwd)),
            Err(ManagedError::InvalidRequest),
            "{cwd}"
        );
    }
    assert!(wt_plan(Path::new(r"\\server\share\My dir")).is_ok());
}

/// The program a real launch's stub records: its working folder, the
/// command line it was given, and each argument after its own path, to
/// `<its own path>.out`, renamed into place once whole. Compiled with
/// `rustc` at test time, so the crate gains no binary. It reads the raw
/// command line from `GetCommandLineW` so a failure shows how the line
/// was quoted.
const STUB_SOURCE: &str = r#"
#[link(name = "kernel32")]
extern "system" {
    fn GetCommandLineW() -> *const u16;
}
fn main() {
    let exe = std::env::current_exe().unwrap();
    let out = exe.with_extension("out");
    let raw = unsafe {
        let p = GetCommandLineW();
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
    };
    let cwd = std::env::current_dir()
        .map(|d| d.into_os_string().into_string().unwrap_or_else(|_| "<not unicode>".into()))
        .unwrap_or_else(|_| "<no cwd>".into());
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.into_string().unwrap_or_else(|_| "<not unicode>".into()))
        .collect();
    let mut body = format!("{cwd}\n{raw}\n{}\n", args.len());
    for arg in args {
        body.push_str(&arg);
        body.push('\0');
    }
    let tmp = exe.with_extension("tmp");
    std::fs::write(&tmp, body).unwrap();
    std::fs::rename(&tmp, &out).unwrap();
}
"#;

/// What a stub recorded.
#[derive(Debug, PartialEq, Eq)]
struct Arrived {
    cwd: String,
    raw: String,
    args: Vec<String>,
}

fn read_arrived(out: &Path) -> Option<Arrived> {
    let text = std::fs::read_to_string(out).ok()?;
    let mut lines = text.splitn(4, '\n');
    let cwd = lines.next()?.to_owned();
    let raw = lines.next()?.to_owned();
    let count: usize = lines.next()?.parse().ok()?;
    let rest = lines.next().unwrap_or("");
    let args: Vec<String> = rest.split('\0').map(str::to_owned).collect();
    // `split` yields one empty piece after the last NUL.
    let args = args[..args.len().saturating_sub(1)].to_vec();
    (args.len() == count).then_some(Arrived { cwd, raw, args })
}

/// Opens each offered terminal installed here with its real launch argv,
/// a stub in place of the helper, and each hostile folder, and checks that
/// the folder and every argument arrived whole. Folders `plan` refuses are
/// listed and must be exactly the expected refusals. Run by
/// `windows/scripts/managed-terminal-launch-smoke.ps1` inside the
/// interactive desktop session; never in CI, which starts no terminal.
/// Each launch leaves a Windows Terminal tab that closes when the stub
/// exits (the default `closeOnExit`).
#[test]
#[ignore = "starts every installed Windows terminal; run on a Windows desktop"]
fn real_windows_terminals_receive_the_folder_and_every_argument() {
    let apps: Vec<(TerminalApp, PathBuf)> = TerminalApp::WINDOWS
        .into_iter()
        .filter_map(|app| Some((app, external_terminal(app.executable()?)?)))
        .collect();
    assert!(
        !apps.is_empty(),
        "no offered terminal is installed here; nothing was tested"
    );
    let root = tempfile::tempdir().unwrap();
    let root = root.path().to_path_buf();
    // The helper's folder and the config folder hold spaces, a quote and
    // non-ASCII text too.
    let stub_dir = root.join("helper dir it's \u{f1}");
    std::fs::create_dir_all(&stub_dir).unwrap();
    let source = root.join("stub.rs");
    std::fs::write(&source, STUB_SOURCE).unwrap();
    let built = root.join("stub.exe");
    let rustc = std::process::Command::new("rustc")
        .args(["-O", "--edition", "2021", "-o"])
        .arg(&built)
        .arg(&source)
        .status()
        .expect("rustc on PATH");
    assert!(rustc.success(), "the stub did not compile");

    // (folder, config folder), named under `root` unless absolute.
    let mut cases: Vec<(PathBuf, PathBuf)> = vec![];
    let config = root.join("config dir 'x' \u{f1}");
    for name in [
        "My Projects\\app",
        "it's 'single' q",
        "a & b ^ c (d) !e! =f, g",
        "100% %USERPROFILE% x",
        "proyecto-a\u{f1}o\\\u{65e5}\u{672c}\u{8a9e} \u{1f642}",
        "-d",
        "back`tick $HOME",
        "a;b",
    ] {
        cases.push((root.join(name), config.clone()));
    }
    // The drive root, whose path ends in a backslash.
    let drive_root = PathBuf::from(format!(
        "{}\\",
        std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into())
    ));
    cases.push((drive_root, config.clone()));
    // A config folder holding `%NAME%`, inside the command Windows Terminal
    // writes for the helper.
    cases.push((root.join("plain"), root.join("cfg %USERNAME% x")));
    // The folders `plan` refuses for Windows Terminal, by name: `;` in the
    // folder, and the case whose config folder holds `%NAME%`.
    let refused_for_wt = ["a;b", "plain"];

    let session = Uuid::new_v4();
    let ticket = "abcdef0123456789";
    let mut started = vec![];
    let mut refused = vec![];
    let mut failed = vec![];
    for (app, program) in &apps {
        for (index, (folder, config)) in cases.iter().enumerate() {
            if !folder.exists() {
                std::fs::create_dir_all(folder).unwrap();
            }
            std::fs::create_dir_all(config).unwrap();
            let stub = stub_dir.join(format!("stub {} {index}.exe", app.id()));
            std::fs::copy(&built, &stub).unwrap();
            let out = stub.with_extension("out");
            let label = format!("{} {folder:?} {config:?}", app.name());
            let plan = match plan(*app, program, &stub, config, folder, session, ticket) {
                Ok(plan) => plan,
                Err(error) => {
                    refused.push((app.name(), folder.file_name().map(|n| n.to_owned())));
                    eprintln!("refused: {label}: {error:?}");
                    continue;
                }
            };
            let mut child = plan
                .command()
                .stdin(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !out.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            let _ = child.try_wait();
            started.push(label.clone());
            let Some(arrived) = read_arrived(&out) else {
                failed.push(format!("{label}: the stub never ran"));
                continue;
            };
            eprintln!("arrived: {label}: command line {:?}", arrived.raw);
            let want: Vec<String> = helper_args(config, session, ticket)
                .into_iter()
                .map(|a| a.into_string().unwrap())
                .collect();
            let want_cwd = folder.to_str().unwrap().to_owned();
            if arrived.cwd != want_cwd || arrived.args != want {
                failed.push(format!(
                    "{label}: got cwd {:?} args {:?} from {:?}; want cwd {want_cwd:?} args {want:?}",
                    arrived.cwd, arrived.args, arrived.raw
                ));
            }
        }
    }
    eprintln!("launched: {started:#?}");
    eprintln!("refused: {refused:#?}");
    assert!(!started.is_empty(), "no terminal was started");
    for (app, _) in &apps {
        let names: Vec<_> = refused
            .iter()
            .filter(|(name, _)| *name == app.name())
            .filter_map(|(_, folder)| folder.as_ref()?.to_str().map(str::to_owned))
            .collect();
        if *app == TerminalApp::WindowsTerminal {
            assert_eq!(names, refused_for_wt, "Windows Terminal's refusals");
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}
