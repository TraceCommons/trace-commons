//! `connect`, `disconnect` and `init` with more than one login per product.
//!
//! Each test runs the real binary on a private HOME with a private `PATH`.
//! Its fake `security` reports an absent item, never this developer's Keychain.
//! The tests read back the files it wrote. What is being pinned is which file each login's
//! client was pointed from, and with which URL: that is what decides whose
//! subscription pays for a request.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

// Each case launches a CLI plus synthetic Keychain subprocesses. Serializing
// those fixtures prevents competing timeout tests from turning process-load
// delays into a false credential failure; daemon concurrency has its own suite.
static CLI_FIXTURE: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Machine {
    _fixture: std::sync::MutexGuard<'static, ()>,
    home: tempfile::TempDir,
}

impl Machine {
    fn new() -> Self {
        let machine = Self {
            _fixture: CLI_FIXTURE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            home: tempfile::tempdir().expect("tempdir"),
        };
        std::fs::create_dir_all(machine.runtime()).expect("mkdir");
        machine.security_script("exit 44");
        machine
    }

    fn home(&self) -> &Path {
        self.home.path()
    }

    fn runtime(&self) -> PathBuf {
        self.home().join("runtime")
    }

    fn security_script(&self, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let bin = self.home().join("bin");
        std::fs::create_dir_all(&bin).expect("mkdir fixture bin");
        let security = bin.join("security");
        std::fs::write(&security, format!("#!/bin/sh\n{body}\n")).expect("write security stub");
        std::fs::set_permissions(security, std::fs::Permissions::from_mode(0o700))
            .expect("executable security stub");
    }

    fn config(&self) -> String {
        std::fs::read_to_string(self.runtime().join("config.toml")).unwrap_or_default()
    }

    fn log_in_claude(&self, dir: &Path, token: &str) {
        std::fs::create_dir_all(dir).expect("mkdir");
        let expires_at = (chrono::Utc::now() + chrono::Duration::hours(8)).timestamp_millis();
        std::fs::write(
            dir.join(".credentials.json"),
            serde_json::json!({
                "claudeAiOauth": {
                    "accessToken": token,
                    "refreshToken": "sk-ant-ort01-unused",
                    "expiresAt": expires_at,
                    "subscriptionType": "max",
                }
            })
            .to_string(),
        )
        .expect("write credentials");
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ironwire"));
        command
            .args(args)
            .env_clear()
            .env("HOME", self.home())
            .env("IRONWIRE_HOME", self.runtime())
            .env("PATH", self.home().join("bin"))
            .stdin(Stdio::null());
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        let output = self.command(args).output().expect("runs");
        assert!(
            output.status.success(),
            "ironwire {args:?} failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn answering(&self, args: &[&str], answer: &str) -> Output {
        use std::io::Write;
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawns");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(answer.as_bytes())
            .expect("answers");
        let output = child.wait_with_output().expect("runs");
        assert!(
            output.status.success(),
            "ironwire {args:?} failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn base_url_in(&self, dir: &Path) -> Option<String> {
        let text = std::fs::read_to_string(dir.join("settings.json")).ok()?;
        let parsed: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        Some(parsed["env"]["ANTHROPIC_BASE_URL"].as_str()?.to_string())
    }

    fn consent(&self) -> serde_json::Value {
        let text = std::fs::read_to_string(self.runtime().join("consent.json")).unwrap_or_default();
        serde_json::from_str(&text).unwrap_or_default()
    }

    /// Consent recorded the way an earlier version did: by id alone, which
    /// is consent to the login in its usual place.
    fn consented_long_ago(&self, id: &str) {
        std::fs::write(
            self.runtime().join("consent.json"),
            serde_json::json!({ id: { "prompt_version": 2, "granted_at": "2026-01-01T00:00:00Z" } })
                .to_string(),
        )
        .expect("write consent");
    }

    /// Consent as this version records it, against the directory it answered.
    fn consented_in(&self, id: &str, location: &Path) {
        std::fs::write(
            self.runtime().join("consent.json"),
            serde_json::json!({
                "claude-sub": { "prompt_version": 2, "granted_at": "2026-01-01T00:00:00Z" },
                id: {
                    "prompt_version": 2,
                    "granted_at": "2026-01-01T00:00:00Z",
                    "location": location.display().to_string(),
                },
            })
            .to_string(),
        )
        .expect("write consent");
    }

    /// `ironwire serve` on this machine, for the commands that ask a running
    /// daemon first.
    fn daemon(&self) -> Daemon {
        use std::io::BufRead;
        let mut child = self
            .command(&["serve", "--port", "0"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawns");
        let mut stdout = std::io::BufReader::new(child.stdout.take().expect("stdout"));
        let mut line = String::new();
        let port = loop {
            line.clear();
            let read = stdout.read_line(&mut line).expect("reads");
            assert!(read > 0, "the daemon exited before it was listening");
            if let Some(port) = line
                .trim()
                .strip_prefix("IronWire listening on http://127.0.0.1:")
            {
                break port.parse().expect("a port");
            }
        };
        Daemon {
            child,
            port,
            _stdout: stdout,
        }
    }

    fn consented(&self) -> Vec<String> {
        let Ok(text) = std::fs::read_to_string(self.runtime().join("consent.json")) else {
            return Vec::new();
        };
        let parsed: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        parsed
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default()
    }
}

/// A running daemon, stopped when dropped.
struct Daemon {
    child: std::process::Child,
    port: u16,
    /// Held open, so whatever the daemon prints later has somewhere to go.
    _stdout: std::io::BufReader<std::process::ChildStdout>,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn backends(config: &str) -> Vec<toml::Value> {
    config
        .parse::<toml::Table>()
        .expect("config.toml parses")
        .get("backends")
        .and_then(toml::Value::as_array)
        .cloned()
        .unwrap_or_default()
}

#[test]
fn connecting_a_login_by_directory_declares_it_and_binds_only_its_claude_code() {
    let machine = Machine::new();
    let work = machine.home().join(".claude-work");
    machine.log_in_claude(&work, "sk-ant-oat01-work");
    let work_arg = work.display().to_string();

    let output = machine.run(&[
        "connect",
        "claude",
        "--config-dir",
        &work_arg,
        "--port",
        "8463",
    ]);
    let said = stdout(&output);

    // Declared, under an id taken from the directory, with the directory
    // written the way it is exported.
    let declared = backends(&machine.config());
    assert_eq!(declared.len(), 1, "{}", machine.config());
    assert_eq!(declared[0]["id"].as_str(), Some("claude-work"));
    assert_eq!(declared[0]["kind"].as_str(), Some("claude-subscription"));
    assert_eq!(declared[0]["config_dir"].as_str(), Some("~/.claude-work"));

    // That login's Claude Code names it; the default one is not touched.
    assert_eq!(
        machine.base_url_in(&work).as_deref(),
        Some("http://127.0.0.1:8463/anthropic/account/claude-work")
    );
    assert!(
        !machine
            .home()
            .join(".claude")
            .join("settings.json")
            .exists()
    );

    // The next step names the login, not the default one.
    assert!(
        said.contains("ironwire connect claude --config-dir ~/.claude-work --subscription"),
        "{said}"
    );
    assert!(said.contains("restart it"), "{said}");

    // A second run finds it declared and changes nothing.
    let before = machine.config();
    let again = stdout(&machine.run(&[
        "connect",
        "claude",
        "--config-dir",
        "~/.claude-work",
        "--port",
        "8463",
    ]));
    assert_eq!(machine.config(), before);
    assert!(!again.contains("config.toml:"), "{again}");
}

/// Today's account mixing, fixed: with `CLAUDE_CONFIG_DIR` exported, the work
/// directory's settings used to be pointed at the plain URL while the default
/// login's token paid for it.
#[test]
fn a_claude_config_dir_in_the_shell_means_that_login() {
    let machine = Machine::new();
    let work = machine.home().join(".claude-work");
    machine.log_in_claude(&work, "sk-ant-oat01-work");

    let output = machine
        .command(&["connect", "claude", "--port", "8463"])
        .env("CLAUDE_CONFIG_DIR", &work)
        .output()
        .expect("runs");
    assert!(output.status.success(), "{output:?}");
    let said = stdout(&output);

    assert!(
        said.contains("CLAUDE_CONFIG_DIR is set to ~/.claude-work"),
        "{said}"
    );
    assert_eq!(
        backends(&machine.config())[0]["id"].as_str(),
        Some("claude-work")
    );
    assert_eq!(
        machine.base_url_in(&work).as_deref(),
        Some("http://127.0.0.1:8463/anthropic/account/claude-work")
    );
}

/// Consent is per login: the question names the directory, and the answer is
/// recorded against that login's id and nobody else's.
#[test]
fn consent_for_one_login_is_asked_and_recorded_under_its_own_id() {
    let machine = Machine::new();
    machine.log_in_claude(&machine.home().join(".claude"), "sk-ant-oat01-personal");
    let work = machine.home().join(".claude-work");
    machine.log_in_claude(&work, "sk-ant-oat01-work");

    let said = stdout(&machine.answering(
        &[
            "connect",
            "claude",
            "--config-dir",
            "~/.claude-work",
            "--subscription",
        ],
        "y\n",
    ));
    assert!(
        said.contains("Enable the Claude subscription in ~/.claude-work?"),
        "{said}"
    );
    let consented = machine.consented();
    assert!(
        consented.iter().any(|id| id == "claude-work"),
        "{consented:?}"
    );
    assert!(
        !consented.iter().any(|id| id == "claude-sub"),
        "{consented:?}"
    );

    // And withdrawn the same way, by id.
    let revoked = stdout(&machine.run(&[
        "disconnect",
        "claude",
        "--subscription",
        "--id",
        "claude-work",
    ]));
    assert!(revoked.contains("Revoked consent"), "{revoked}");
    assert!(
        !machine.consented().iter().any(|id| id == "claude-work"),
        "{:?}",
        machine.consented()
    );
}

/// `CODEX_HOME` already means the default login — to the daemon that inherits
/// it, and to the hermetic tests that plant it — so it stays that, and says
/// how to declare it as a second login instead.
#[test]
fn a_codex_home_in_the_shell_still_means_the_default_login() {
    let machine = Machine::new();
    let codex_home = machine.home().join("codex-default");
    std::fs::create_dir_all(&codex_home).expect("mkdir");

    let output = machine
        .command(&["connect", "codex", "--port", "8463"])
        .env("CODEX_HOME", &codex_home)
        .output()
        .expect("runs");
    assert!(output.status.success(), "{output:?}");
    let said = stdout(&output);

    assert!(
        said.contains("treats it as your default Codex login"),
        "{said}"
    );
    assert!(said.contains("--config-dir"), "{said}");
    assert!(
        backends(&machine.config()).is_empty(),
        "{}",
        machine.config()
    );
    let written = std::fs::read_to_string(codex_home.join("config.toml")).expect("wired");
    assert!(
        written.contains("base_url = \"http://127.0.0.1:8463/openai/v1\""),
        "{written}"
    );
}

#[test]
fn a_codex_home_named_on_the_command_line_is_its_own_login() {
    let machine = Machine::new();
    let work = machine.home().join(".codex-work");
    std::fs::create_dir_all(&work).expect("mkdir");

    let said = stdout(&machine.run(&[
        "connect",
        "codex",
        "--config-dir",
        "~/.codex-work",
        "--port",
        "8463",
    ]));

    let declared = backends(&machine.config());
    assert_eq!(declared[0]["id"].as_str(), Some("codex-work"));
    assert_eq!(declared[0]["kind"].as_str(), Some("codex-subscription"));
    let written = std::fs::read_to_string(work.join("config.toml")).expect("wired");
    assert!(
        written.contains("base_url = \"http://127.0.0.1:8463/openai/account/codex-work/v1\""),
        "{written}"
    );
    // Nobody is logged in there yet, and the way to fix that names the home.
    assert!(
        said.contains("CODEX_HOME=~/.codex-work codex login"),
        "{said}"
    );
}

/// An entry named by hand is found by its directory, so the command a status
/// screen prints for it leads back to it rather than to a duplicate.
#[test]
fn a_login_named_by_hand_is_found_again_by_its_directory() {
    let machine = Machine::new();
    let work = machine.home().join(".claude-work");
    machine.log_in_claude(&work, "sk-ant-oat01-work");
    std::fs::write(
        machine.runtime().join("config.toml"),
        "# mine\n[[backends]]\nid = \"work\"\nkind = \"claude-subscription\"\n\
         config_dir = \"~/.claude-work\"\n",
    )
    .expect("write config");
    let before = machine.config();

    machine.run(&[
        "connect",
        "claude",
        "--config-dir",
        "~/.claude-work",
        "--port",
        "8463",
    ]);
    assert_eq!(machine.config(), before);
    assert_eq!(
        machine.base_url_in(&work).as_deref(),
        Some("http://127.0.0.1:8463/anthropic/account/work")
    );
}

#[test]
fn disconnecting_a_login_unwires_only_its_own_client() {
    let machine = Machine::new();
    let personal = machine.home().join(".claude");
    machine.log_in_claude(&personal, "sk-ant-oat01-personal");
    let work = machine.home().join(".claude-work");
    machine.log_in_claude(&work, "sk-ant-oat01-work");

    machine.run(&["connect", "claude", "--port", "8463"]);
    machine.run(&[
        "connect",
        "claude",
        "--config-dir",
        "~/.claude-work",
        "--port",
        "8463",
    ]);
    assert!(machine.base_url_in(&personal).is_some());
    assert!(machine.base_url_in(&work).is_some());

    machine.run(&["disconnect", "claude", "--config-dir", "~/.claude-work"]);
    assert_eq!(machine.base_url_in(&work), None);
    assert_eq!(
        machine.base_url_in(&personal).as_deref(),
        Some("http://127.0.0.1:8463/anthropic")
    );
}

/// `init` asks one question about every login it found, names each by where
/// it lives, and points each login's Claude Code at its own URL.
#[test]
fn init_asks_once_about_every_login_and_wires_each_to_its_own_url() {
    let machine = Machine::new();
    machine.log_in_claude(&machine.home().join(".claude"), "sk-ant-oat01-personal");
    machine.log_in_claude(&machine.home().join(".claude-work"), "sk-ant-oat01-work");
    std::fs::write(
        machine.runtime().join("config.toml"),
        "[updates]\ncheck = false\n\n[[backends]]\nid = \"claude-work\"\n\
         kind = \"claude-subscription\"\nconfig_dir = \"~/.claude-work\"\n",
    )
    .expect("write config");

    // A port nothing listens on, so the dry run reports instead of finding a
    // daemon that belongs to whoever is running these tests.
    let said = stdout(&machine.run(&["init", "--dry-run", "--no-service", "--port", "9"]));

    let found = |location: &str| format!("found    {:<28}{location} ·", "Claude Code login");
    assert!(said.contains(&found("~/.claude")), "{said}");
    assert!(said.contains(&found("~/.claude-work")), "{said}");
    assert!(
        said.contains("Use the Claude subscriptions in ~/.claude and ~/.claude-work? [Y/n]"),
        "{said}"
    );
    assert!(!said.contains("Anthropic and Anthropic"), "{said}");
    assert!(
        said.contains("`http://127.0.0.1:9/anthropic/account/claude-work`"),
        "{said}"
    );
    assert!(
        said.contains("`http://127.0.0.1:9/anthropic` (added)"),
        "{said}"
    );
    // A dry run, and not a terminal: nothing was written or granted.
    assert!(
        machine
            .base_url_in(&machine.home().join(".claude-work"))
            .is_none()
    );
    assert!(machine.consented().is_empty());
}

/// `eval "$(ironwire env)"` in a shell that runs a declared login's Claude
/// Code points it at that login, not at the default one.
#[test]
fn the_environment_for_a_declared_login_names_that_login() {
    let machine = Machine::new();
    let work = machine.home().join(".claude-work");
    std::fs::write(
        machine.runtime().join("config.toml"),
        "[[backends]]\nid = \"claude-work\"\nkind = \"claude-subscription\"\n\
         config_dir = \"~/.claude-work\"\n",
    )
    .expect("write config");

    let output = machine
        .command(&["env", "--shell", "bash", "--port", "8463"])
        .env("CLAUDE_CONFIG_DIR", &work)
        .output()
        .expect("runs");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        stdout(&output).trim(),
        "export ANTHROPIC_BASE_URL=http://127.0.0.1:8463/anthropic/account/claude-work"
    );

    let plain = stdout(&machine.run(&["env", "--shell", "bash", "--port", "8463"]));
    assert_eq!(
        plain.trim(),
        "export ANTHROPIC_BASE_URL=http://127.0.0.1:8463/anthropic"
    );
}

/// A `CLAUDE_CONFIG_DIR` no login declares is pointed at `ironwire connect
/// claude`, and the note says what that will do: `~/.claude` itself moves the
/// default login there, so "adds it" would promise a second login connect
/// never makes.
#[test]
fn a_note_about_an_undeclared_claude_config_dir_says_what_connect_will_do() {
    let machine = Machine::new();
    let note_for = |dir: PathBuf| {
        let output = machine
            .command(&["env", "--shell", "bash", "--port", "8463"])
            .env("CLAUDE_CONFIG_DIR", dir)
            .output()
            .expect("runs");
        assert!(output.status.success(), "{output:?}");
        stderr(&output)
    };

    // Only where Claude Code keys a Keychain item to the exact string is an
    // exported `~/.claude` a login of its own; elsewhere it reads the default
    // login's credentials file, and is the default login.
    let usual = note_for(machine.home().join(".claude"));
    if cfg!(target_os = "macos") {
        assert!(
            usual.contains(
                "`ironwire connect claude` moves your default login (`claude-sub`) to it"
            ),
            "{usual}"
        );
    } else {
        assert!(!usual.contains("CLAUDE_CONFIG_DIR"), "{usual}");
    }
    let other = note_for(machine.home().join(".claude-other"));
    assert!(
        other.contains("`ironwire connect claude` adds it"),
        "{other}"
    );
}

/// The plain URL belongs to the default login in its usual place. Moving the
/// default wires its new Claude Code with the URL naming it — and the Claude
/// Code it moved away from, still on the plain URL, is served by no
/// subscription from then on, which the user is told along with the command
/// that takes it off. The old consent answered for the old login, so the
/// moved one is asked about by its new directory.
#[test]
fn moving_the_default_login_warns_about_the_client_still_pointed_at_the_plain_url() {
    let machine = Machine::new();
    let personal = machine.home().join(".claude");
    let main = machine.home().join(".claude-main");
    machine.log_in_claude(&main, "sk-ant-oat01-main");
    machine.consented_long_ago("claude-sub");
    machine.run(&["connect", "claude", "--port", "8463"]);
    assert_eq!(
        machine.base_url_in(&personal).as_deref(),
        Some("http://127.0.0.1:8463/anthropic")
    );

    let said = stdout(&machine.answering(
        &[
            "connect",
            "claude",
            "--config-dir",
            "~/.claude-main",
            "--id",
            "claude-sub",
            "--subscription",
            "--port",
            "8463",
        ],
        "y\n",
    ));

    let declared = backends(&machine.config());
    assert_eq!(declared.len(), 1, "{}", machine.config());
    assert_eq!(declared[0]["id"].as_str(), Some("claude-sub"));
    assert_eq!(declared[0]["config_dir"].as_str(), Some("~/.claude-main"));
    assert_eq!(
        machine.base_url_in(&main).as_deref(),
        Some("http://127.0.0.1:8463/anthropic/account/claude-sub")
    );
    assert!(
        said.contains("This moves your default Claude login"),
        "{said}"
    );
    assert!(
        said.contains("~/.claude/settings.json still points Claude Code at IronWire's plain URL"),
        "{said}"
    );
    assert!(
        said.contains("`ironwire disconnect claude --config-dir ~/.claude`"),
        "{said}"
    );
    // Left alone: the note is the answer, not an edit nobody asked for.
    assert_eq!(
        machine.base_url_in(&personal).as_deref(),
        Some("http://127.0.0.1:8463/anthropic")
    );

    assert!(
        said.contains("Enable the Claude subscription in ~/.claude-main?"),
        "{said}"
    );
    assert_eq!(
        machine.consent()["claude-sub"]["location"].as_str(),
        Some(main.display().to_string().as_str()),
        "{}",
        machine.consent()
    );

    // The command it named takes that Claude Code off, and only that one.
    machine.run(&["disconnect", "claude", "--config-dir", "~/.claude"]);
    assert_eq!(machine.base_url_in(&personal), None);
    assert_eq!(
        machine.base_url_in(&main).as_deref(),
        Some("http://127.0.0.1:8463/anthropic/account/claude-sub")
    );
}

/// `~/.claude` and `~/.codex` are the default login's own directories. A
/// second login there would share the default's settings file, so naming one
/// moves the default there — and asking for a second login there by name is
/// refused with the command that means it.
#[test]
fn naming_the_default_directory_moves_the_default_login_rather_than_adding_one() {
    let machine = Machine::new();
    let personal = machine.home().join(".claude");
    std::fs::create_dir_all(&personal).expect("mkdir");

    let refused = machine
        .command(&[
            "connect",
            "claude",
            "--config-dir",
            "~/.claude",
            "--id",
            "claude-main",
        ])
        .output()
        .expect("runs");
    assert!(!refused.status.success(), "{refused:?}");
    let why = stderr(&refused);
    assert!(
        why.contains("cannot be a second login called `claude-main`"),
        "{why}"
    );
    assert!(
        why.contains("`ironwire connect claude --config-dir ~/.claude`"),
        "{why}"
    );
    assert_eq!(machine.config(), "");

    let said = stdout(&machine.run(&[
        "connect",
        "claude",
        "--config-dir",
        "~/.claude",
        "--port",
        "8463",
    ]));
    let declared = backends(&machine.config());
    assert_eq!(declared.len(), 1, "{}", machine.config());
    assert_eq!(declared[0]["id"].as_str(), Some("claude-sub"));
    assert_eq!(declared[0]["config_dir"].as_str(), Some("~/.claude"));
    assert_eq!(
        machine.base_url_in(&personal).as_deref(),
        Some("http://127.0.0.1:8463/anthropic/account/claude-sub")
    );
    assert!(said.contains("rather than adding a second login"), "{said}");

    machine.run(&[
        "connect",
        "codex",
        "--config-dir",
        "~/.codex",
        "--port",
        "8463",
    ]);
    let declared = backends(&machine.config());
    assert_eq!(declared.len(), 2, "{}", machine.config());
    assert_eq!(declared[1]["id"].as_str(), Some("codex-sub"));
    assert_eq!(declared[1]["config_dir"].as_str(), Some("~/.codex"));
}

/// Filesystem aliases can name different macOS credentials, even though they
/// write the same settings file. Connecting must refuse rather than choose one.
#[test]
#[cfg(target_os = "macos")]
fn a_shell_using_an_alias_of_a_declared_claude_directory_is_refused() {
    let machine = Machine::new();
    let work = machine.home().join(".claude-work");
    machine.log_in_claude(&work, "sk-ant-oat01-work");
    std::fs::write(machine.runtime().join("config.toml"),
        "[[backends]]\nid = \"claude-work\"\nkind = \"claude-subscription\"\nconfig_dir = \"~/.claude-work\"\n").expect("config");
    let before = machine.config();
    for typed in [
        format!("{}/", work.display()),
        format!("{}//.claude-work", machine.home().display()),
    ] {
        let output = machine
            .command(&["connect", "claude"])
            .env("CLAUDE_CONFIG_DIR", &typed)
            .output()
            .expect("runs");
        assert!(
            !output.status.success(),
            "alias accepted: {}",
            stdout(&output)
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("CLAUDE_CONFIG_DIR"));
        assert_eq!(machine.config(), before);
        assert!(!work.join("settings.json").exists());
    }
}

#[test]
fn init_never_wires_a_disabled_default_login_at_a_bound_url() {
    let machine = Machine::new();
    let work = machine.home().join(".claude-work");
    std::fs::write(machine.runtime().join("config.toml"),
        "[[backends]]\nid = \"claude-sub\"\nkind = \"claude-subscription\"\nconfig_dir = \"~/.claude-work\"\nenabled = false\n").expect("config");
    let output = machine
        .command(&["init", "--no-service"])
        .env("ANTHROPIC_API_KEY", "test-key")
        .output()
        .expect("runs");
    assert!(output.status.success(), "{}", stdout(&output));
    assert!(
        !work.join("settings.json").exists(),
        "disabled login was wired"
    );
    assert!(!stdout(&output).contains("wired    Claude Code (~/.claude-work)"));
}

/// With a `CLAUDE_CONFIG_DIR` no login uses, `init` wires only the default
/// login's Claude Code — never that directory with the plain URL, which the
/// default login's token would then serve — and does not claim the `claude`
/// this shell starts goes through IronWire, because it reads the other file.
#[test]
fn init_with_an_undeclared_claude_config_dir_wires_only_the_default_login() {
    let machine = Machine::new();
    let personal = machine.home().join(".claude");
    let work = machine.home().join(".claude-work");
    machine.log_in_claude(&personal, "sk-ant-oat01-personal");
    machine.log_in_claude(&work, "sk-ant-oat01-work");
    std::fs::write(
        machine.runtime().join("config.toml"),
        "[updates]\ncheck = false\n",
    )
    .expect("write config");

    let output = machine
        .command(&["init", "--dry-run", "--no-service", "--port", "9"])
        .env("CLAUDE_CONFIG_DIR", &work)
        .output()
        .expect("runs");
    assert!(output.status.success(), "{output:?}");
    let said = stdout(&output);

    assert!(
        said.contains("a login IronWire does not know yet"),
        "{said}"
    );
    assert!(said.contains("`ironwire connect claude` adds it"), "{said}");
    assert!(
        said.contains(&format!(
            "Writing {}",
            personal.join("settings.json").display()
        )),
        "{said}"
    );
    assert!(!said.contains(".claude-work/settings.json"), "{said}");
    assert!(
        !said.contains("run `claude` — it goes through IronWire now"),
        "{said}"
    );
    assert!(said.contains("Next: `ironwire connect claude`"), "{said}");
}

/// `claude` on PATH is the Claude Code this shell starts, and with an
/// undeclared `CLAUDE_CONFIG_DIR` it does not read `~/.claude`. So `init`
/// does not create a `~/.claude` for it, and makes connecting that login the
/// next step instead of saying it is done.
#[test]
fn init_does_not_wire_a_default_claude_code_that_only_its_path_entry_suggests() {
    let machine = Machine::new();
    let bin = machine.home().join("bin");
    std::fs::create_dir_all(&bin).expect("mkdir");
    std::fs::write(bin.join("claude"), "#!/bin/sh\n").expect("stub");
    let elsewhere = machine.home().join(".config").join("claude");
    std::fs::create_dir_all(&elsewhere).expect("mkdir");

    let output = machine
        .command(&["init", "--dry-run", "--no-service", "--port", "9"])
        .env("CLAUDE_CONFIG_DIR", &elsewhere)
        .env("PATH", &bin)
        .env("ANTHROPIC_API_KEY", "sk-ant-api03-INERT")
        .output()
        .expect("runs");
    assert!(output.status.success(), "{output:?}");
    let said = stdout(&output);

    let default = machine.home().join(".claude").join("settings.json");
    assert!(
        !said.contains(&default.display().to_string()),
        "wired a settings file nothing here reads:\n{said}"
    );
    assert!(!said.contains("goes through IronWire now"), "{said}");
    assert!(!said.contains("No coding agent found here yet"), "{said}");
    assert!(said.contains("Next: `ironwire connect claude`"), "{said}");
}

/// The default Claude login is the unsuffixed Keychain item and
/// `~/.claude/.credentials.json`, whatever `CLAUDE_CONFIG_DIR` says. The CLI
/// and the daemon build it through one constructor, so a run with the
/// variable pointed at a logged-in directory, and nothing in `~/.claude`,
/// finds no default login to report or ask about.
#[test]
fn the_default_claude_login_never_follows_claude_config_dir() {
    let machine = Machine::new();
    let work = machine.home().join(".claude-work");
    machine.log_in_claude(&work, "sk-ant-oat01-work");

    let output = machine
        .command(&["init", "--dry-run", "--no-service", "--port", "9"])
        .env("CLAUDE_CONFIG_DIR", &work)
        .env("ANTHROPIC_API_KEY", "sk-ant-api03-INERT")
        .output()
        .expect("runs");
    assert!(output.status.success(), "{output:?}");
    let said = stdout(&output);

    assert!(!said.contains("Claude Code login"), "{said}");
    assert!(!said.contains("Use the Claude subscription"), "{said}");
    assert!(
        said.contains("a login IronWire does not know yet"),
        "{said}"
    );
}

/// `resolve` serves `connect` and `disconnect` alike, and a mistake made while
/// disconnecting is answered with a disconnect. Told to run `connect`, the
/// user would wire the login back up instead of taking it off.
#[test]
fn a_mistaken_disconnect_is_told_a_disconnect_command() {
    let machine = Machine::new();
    std::fs::write(
        machine.runtime().join("config.toml"),
        "[[backends]]\nid = \"claude-work\"\nkind = \"claude-subscription\"\n\
         config_dir = \"~/.claude-work\"\n",
    )
    .expect("write config");

    let wrong_product = machine
        .command(&[
            "disconnect",
            "codex",
            "--subscription",
            "--id",
            "claude-work",
        ])
        .output()
        .expect("runs");
    assert!(!wrong_product.status.success());
    let why = stderr(&wrong_product);
    assert!(
        why.contains("run `ironwire disconnect claude --id claude-work --subscription`"),
        "{why}"
    );
    assert!(!why.contains("ironwire connect"), "{why}");

    let misspelled = machine
        .command(&[
            "disconnect",
            "claude",
            "--subscription",
            "--id",
            "claude-wrk",
        ])
        .output()
        .expect("runs");
    assert!(!misspelled.status.success());
    let why = stderr(&misspelled);
    assert!(why.contains("`claude-work` (~/.claude-work)"), "{why}");
    assert!(!why.contains("ironwire connect"), "{why}");
    assert!(!why.contains("To add one"), "{why}");
}

/// In a shell with `CLAUDE_CONFIG_DIR` set, `disconnect claude
/// --subscription` is about that directory's login. Consent given from the
/// same shell before that meant a login of its own was recorded for the
/// default one — so finding nothing to withdraw, it says the default still
/// holds consent, and names the command that withdraws it.
#[test]
fn a_shell_login_with_no_consent_names_the_default_login_that_still_has_it() {
    let machine = Machine::new();
    machine.consented_long_ago("claude-sub");
    let elsewhere = machine.home().join(".config").join("claude");

    let output = machine
        .command(&["disconnect", "claude", "--subscription"])
        .env("CLAUDE_CONFIG_DIR", &elsewhere)
        .output()
        .expect("runs");
    assert!(output.status.success(), "{output:?}");
    let said = stdout(&output);
    assert!(
        said.contains("No consent is recorded for the Claude subscription (~/.config/claude)"),
        "{said}"
    );
    assert!(
        said.contains("Your default login (`claude-sub`, ~/.claude) still holds consent"),
        "{said}"
    );
    assert!(
        said.contains("`ironwire disconnect claude --id claude-sub --subscription`"),
        "{said}"
    );
    assert_eq!(machine.consented(), ["claude-sub"]);

    let output = machine
        .command(&[
            "disconnect",
            "claude",
            "--id",
            "claude-sub",
            "--subscription",
        ])
        .env("CLAUDE_CONFIG_DIR", &elsewhere)
        .output()
        .expect("runs");
    assert!(output.status.success(), "{output:?}");
    assert!(machine.consented().is_empty(), "{:?}", machine.consented());
}

/// Withdrawing the login a `CLAUDE_CONFIG_DIR` names leaves the default
/// login's consent where it is, whether the shell's login held a record for
/// its directory, for another one, or none — so the default's surviving yes
/// is named, with the command that withdraws it, every time.
#[test]
fn withdrawing_a_shell_login_that_had_consent_still_names_the_default_that_keeps_it() {
    let machine = Machine::new();
    let elsewhere = machine.home().join(".config").join("claude");
    let disconnect = || {
        let output = machine
            .command(&["disconnect", "claude", "--subscription"])
            .env("CLAUDE_CONFIG_DIR", &elsewhere)
            .output()
            .expect("runs");
        assert!(output.status.success(), "{output:?}");
        stdout(&output)
    };
    let names_the_default = |said: &str| {
        assert!(
            said.contains("Your default login (`claude-sub`, ~/.claude) still holds consent"),
            "{said}"
        );
        assert!(
            said.contains("`ironwire disconnect claude --id claude-sub --subscription`"),
            "{said}"
        );
    };

    machine.consented_in("claude", &elsewhere);
    let said = disconnect();
    assert!(
        said.contains("Revoked consent for the Claude subscription (~/.config/claude)"),
        "{said}"
    );
    names_the_default(&said);
    assert_eq!(machine.consented(), ["claude-sub"]);

    // A record given while the login read some other directory goes too, and
    // the default is still named.
    machine.consented_in("claude", &machine.home().join(".config").join("old"));
    let said = disconnect();
    assert!(
        said.contains("Revoked the consent recorded for `claude`"),
        "{said}"
    );
    names_the_default(&said);
    assert_eq!(machine.consented(), ["claude-sub"]);
}

/// On macOS, `CLAUDE_CONFIG_DIR=~/.claude` keeps its login in a Keychain
/// item apart from the one plain `claude` uses, but both read one
/// `settings.json` — and that file is all IronWire knows a client's login by.
/// So moving the default login there says, before writing anything, that a
/// plain `claude` will be served by it too. Elsewhere it is one login.
#[test]
fn moving_the_default_login_into_its_own_directory_says_plain_claude_shares_it_on_macos() {
    let machine = Machine::new();
    let personal = machine.home().join(".claude");
    std::fs::create_dir_all(&personal).expect("mkdir");
    let warning = "Any `claude` started without CLAUDE_CONFIG_DIR reads the same settings.json \
                   and will be served by this login, `claude-sub`";

    let named = stdout(&machine.run(&[
        "connect",
        "claude",
        "--config-dir",
        "~/.claude",
        "--port",
        "8463",
        "--dry-run",
    ]));
    let exported = machine
        .command(&["connect", "claude", "--port", "8463", "--dry-run"])
        .env("CLAUDE_CONFIG_DIR", &personal)
        .output()
        .expect("runs");
    assert!(exported.status.success(), "{exported:?}");
    let exported = stdout(&exported);

    for said in [named, exported] {
        assert!(said.contains("rather than adding a second login"), "{said}");
        let first_write = said.find("Writing ").expect("would write");
        if cfg!(target_os = "macos") {
            let warned = said.find(warning).unwrap_or_else(|| panic!("{said}"));
            assert!(warned < first_write, "warned after writing:\n{said}");
        } else {
            assert!(!said.contains(warning), "{said}");
        }
    }
    assert_eq!(machine.config(), "");
}

/// `init` in a shell that exports `CLAUDE_CONFIG_DIR=~/.claude` says one
/// thing about the `claude` it starts. Where that is the default login itself
/// it goes through IronWire now; on macOS it is a login of its own that reads
/// the default's settings, so connecting it comes next — and "it goes through
/// IronWire now" would claim it is done.
#[test]
fn init_with_claude_config_dir_set_to_the_default_directory_says_one_thing_about_claude() {
    let machine = Machine::new();
    let personal = machine.home().join(".claude");
    machine.log_in_claude(&personal, "sk-ant-oat01-personal");
    std::fs::write(
        machine.runtime().join("config.toml"),
        "[updates]\ncheck = false\n",
    )
    .expect("write config");

    let output = machine
        .command(&["init", "--dry-run", "--no-service", "--port", "9"])
        .env("CLAUDE_CONFIG_DIR", &personal)
        .output()
        .expect("runs");
    assert!(output.status.success(), "{output:?}");
    let said = stdout(&output);

    let done = said.contains("run `claude` — it goes through IronWire now");
    let next = said.contains("Next: `ironwire connect claude`");
    assert!(done != next, "exactly one of the two:\n{said}");
    if cfg!(target_os = "macos") {
        assert!(next, "{said}");
        assert!(
            said.contains("not the Keychain item plain `claude` uses"),
            "{said}"
        );
    } else {
        assert!(done, "{said}");
        assert!(!said.contains("a login IronWire does not know"), "{said}");
    }
}

/// With an undeclared `CLAUDE_CONFIG_DIR` and no `~/.claude`, `ironwire init`
/// leaves the default login's file alone — so `doctor` must not send the user
/// there. It names the file it checked and the command that connects the
/// Claude Code this shell actually starts, once.
#[test]
fn doctor_points_a_shell_with_only_an_undeclared_claude_config_dir_at_connect_not_init() {
    let machine = Machine::new();
    std::fs::write(
        machine.runtime().join("config.toml"),
        "[updates]\ncheck = false\n",
    )
    .expect("write config");
    let elsewhere = machine.home().join(".config").join("claude");
    std::fs::create_dir_all(&elsewhere).expect("mkdir");
    let daemon = machine.daemon();

    let output = machine
        .command(&["doctor", "--port", &daemon.port.to_string()])
        .env("CLAUDE_CONFIG_DIR", &elsewhere)
        .output()
        .expect("runs");
    assert!(output.status.success(), "{output:?}");
    let said = stdout(&output);

    assert!(
        said.contains("~/.claude/settings.json does not point here"),
        "{said}"
    );
    assert!(said.contains("fix:  ironwire connect claude"), "{said}");
    assert!(!said.contains("ironwire init"), "{said}");
    assert_eq!(
        said.matches("fix:  ironwire connect claude").count(),
        1,
        "{said}"
    );
}

/// A default login moved with `config_dir` leaves the client it moved away
/// from on the plain URL, which no subscription serves any more. `doctor`
/// flags that client for both products, naming the file and the command that
/// takes it off.
#[test]
fn doctor_flags_each_client_a_moved_default_login_left_on_the_plain_url() {
    let machine = Machine::new();
    std::fs::write(
        machine.runtime().join("config.toml"),
        "[updates]\ncheck = false\n\n\
         [[backends]]\nid = \"claude-sub\"\nkind = \"claude-subscription\"\n\
         config_dir = \"~/.claude-main\"\n\n\
         [[backends]]\nid = \"codex-sub\"\nkind = \"codex-subscription\"\n\
         config_dir = \"~/.codex-main\"\n",
    )
    .expect("write config");
    let daemon = machine.daemon();
    let port = daemon.port;

    let claude = machine.home().join(".claude");
    std::fs::create_dir_all(&claude).expect("mkdir");
    std::fs::write(
        claude.join("settings.json"),
        format!(r#"{{"env":{{"ANTHROPIC_BASE_URL":"http://127.0.0.1:{port}/anthropic"}}}}"#),
    )
    .expect("write settings");
    let codex = machine.home().join(".codex");
    std::fs::create_dir_all(&codex).expect("mkdir");
    std::fs::write(
        codex.join("config.toml"),
        format!(
            "model_provider = \"ironwire\"\n\n[model_providers.ironwire]\n\
             base_url = \"http://127.0.0.1:{port}/openai/v1\"\n"
        ),
    )
    .expect("write codex config");

    let said = stdout(&machine.run(&["doctor", "--port", &port.to_string()]));

    for (file, id, moved_to, undo) in [
        (
            "~/.claude/settings.json",
            "claude-sub",
            "~/.claude-main",
            "ironwire disconnect claude --config-dir ~/.claude",
        ),
        (
            "~/.codex/config.toml",
            "codex-sub",
            "~/.codex-main",
            "ironwire disconnect codex --config-dir ~/.codex",
        ),
    ] {
        assert!(
            said.contains(&format!(
                "{file} points here, and no subscription serves the plain URL since `{id}` \
                 moved to {moved_to}"
            )),
            "{said}"
        );
        assert!(said.contains(&format!("fix:  {undo}\n")), "{said}");
    }
}

/// `doctor` probes through the daemon, which sends nothing for a login
/// awaiting consent. That login is not a failed backend: it is named once,
/// with the command that grants it, and the count of failures leaves it out.
#[test]
fn doctor_reports_a_login_awaiting_consent_once_and_not_as_a_failure() {
    use std::io::{Read, Write};
    let provider = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let provider_url = format!("http://{}", provider.local_addr().expect("addr"));
    std::thread::spawn(move || {
        for stream in provider.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
            }
            let body = r#"{"data":[]}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });

    let machine = Machine::new();
    let home = machine.home().join(".claude-home");
    let work = machine.home().join(".claude-work");
    machine.log_in_claude(&home, "sk-ant-oat01-home");
    machine.log_in_claude(&work, "sk-ant-oat01-work");
    std::fs::write(
        machine.runtime().join("config.toml"),
        format!(
            "[updates]\ncheck = false\n\n\
             [[backends]]\nid = \"nearai\"\nkind = \"nearai\"\nenabled = false\n\n\
             [[backends]]\nid = \"claude-sub\"\nkind = \"claude-subscription\"\nenabled = false\n\n\
             [[backends]]\nid = \"claude-home\"\nkind = \"claude-subscription\"\n\
             config_dir = \"{}\"\nbase_url = \"{provider_url}\"\n\n\
             [[backends]]\nid = \"claude-work\"\nkind = \"claude-subscription\"\n\
             config_dir = \"{}\"\nbase_url = \"{provider_url}\"\n",
            home.display(),
            work.display(),
        ),
    )
    .expect("write config");
    machine.consented_in("claude-home", &home);
    let daemon = machine.daemon();

    let said = stdout(&machine.run(&["doctor", "--port", &daemon.port.to_string()]));

    let work_lines: Vec<&str> = said
        .lines()
        .filter(|line| line.starts_with("claude-work "))
        .collect();
    assert_eq!(work_lines.len(), 1, "{said}");
    assert!(work_lines[0].contains("awaiting consent"), "{said}");
    assert!(
        work_lines[0].contains("--config-dir ~/.claude-work --subscription"),
        "{said}"
    );
    assert!(
        said.lines()
            .any(|line| line.starts_with("claude-home ") && line.contains(" ok — ")),
        "{said}"
    );
    assert!(!said.contains("FAILED"), "{said}");
    assert!(said.contains("All connected backends answered."), "{said}");
}

/// A stale file is not an alternative account when the active Keychain cannot
/// be read, including when the security process stops answering.
#[cfg(target_os = "macos")]
#[test]
fn a_failed_or_stalled_keychain_lookup_never_grants_consent_to_a_stale_file_login() {
    for script in ["exit 1", "exec /bin/sleep 10"] {
        let machine = Machine::new();
        machine.security_script(script);
        let work = machine.home().join(".claude-work");
        machine.log_in_claude(&work, "sk-ant-oat01-OLD");
        let started = std::time::Instant::now();
        let output = machine
            .command(&[
                "connect",
                "claude",
                "--config-dir",
                "~/.claude-work",
                "--subscription",
            ])
            .output()
            .expect("runs");
        let why = format!("{}{}", stdout(&output), stderr(&output));
        assert!(why.contains("unlock your macOS Keychain"), "{why}");
        assert!(!why.contains("sk-ant-oat01-OLD"), "{why}");
        assert!(machine.consented().is_empty(), "{:?}", machine.consented());
        assert!(!why.contains("Enable the Claude subscription"), "{why}");
        // Resolving the declaration and checking the selected login can each
        // consult Keychain; both lookups have their own five-second deadline.
        assert!(started.elapsed() < std::time::Duration::from_secs(20));
    }
}
