//! Facts: how to spell a command in this folder, learned from repeated
//! failures, and told to the next sessions at their start.
//!
//! One kind of fact, and no model call. When a session runs a program and
//! the shell says it is not found, and later in the same session the same
//! program runs through a launcher (`python -m pytest`, `npx tsc`,
//! `uv run pytest`), that is one confirmation of "run `pytest` as `python -m
//! pytest` here". A fact is told once three sessions have confirmed it, and
//! dropped once two sessions run the bare program fine with no confirmation
//! between them.
//!
//! Only program names and launcher words are kept: every kept word is a
//! plain word (letters, digits, `.`, `_`, `-`), never a path or an argument.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use super::signals::{CallKind, ToolCall};

/// Sessions that must confirm a fact before it is told.
pub const CONFIRMATIONS: usize = 3;
/// Sessions, with no confirmation between, that must contradict a fact
/// before it is dropped.
pub const CONTRADICTIONS: u32 = 2;
/// The most facts told at a session's start.
pub const MAX_TOLD: usize = 8;
/// The most facts kept per folder, told or forming.
pub const MAX_KEPT: usize = 32;
/// The most launcher words before the program.
const MAX_PREFIX: usize = 3;

/// Programs that run another program.
const LAUNCHERS: &[&str] = &[
    "python", "python3", "py", "uv", "uvx", "poetry", "pipenv", "pdm", "hatch", "rye", "npx",
    "pnpm", "pnpx", "yarn", "bun", "bunx", "npm", "corepack", "bundle", "go", "cargo", "dotnet",
    "mise", "asdf",
];

/// What one session showed, folded call by call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observations {
    /// Programs the shell said were not found.
    pub missing: BTreeSet<String>,
    /// A missing program that then ran through a launcher: (program, launcher words).
    pub repairs: BTreeSet<(String, String)>,
    /// Programs that ran bare without a not-found.
    pub bare_worked: BTreeSet<String>,
}

impl Observations {
    /// Fold one finished call.
    pub fn record(&mut self, call: &ToolCall) {
        let CallKind::Command { command } = &call.kind else {
            return;
        };
        for words in segments(command) {
            let Some(program) = words.first().copied() else {
                continue;
            };
            if not_found(&call.output, program) {
                self.missing.insert(program.to_owned());
                continue;
            }
            self.bare_worked.insert(program.to_owned());
            if !LAUNCHERS.contains(&program) {
                continue;
            }
            for position in 1..=MAX_PREFIX.min(words.len().saturating_sub(1)) {
                let target = words[position];
                if self.missing.contains(target) && !not_found(&call.output, target) {
                    let prefix = words[..position].join(" ");
                    self.repairs.insert((target.to_owned(), prefix));
                    break;
                }
            }
        }
    }
}

/// The plain words of each simple command in `command`, after any leading
/// `NAME=value` assignments. A segment whose words are not all plain is cut
/// at the first one that is not, so a path never becomes a program.
fn segments(command: &str) -> Vec<Vec<&str>> {
    command
        .split(['\n', ';', '|', '&'])
        .map(|segment| {
            segment
                .split_whitespace()
                .skip_while(|word| word.contains('='))
                .take_while(|word| plain(word))
                .collect::<Vec<_>>()
        })
        .filter(|words| !words.is_empty())
        .collect()
}

fn plain(word: &str) -> bool {
    !word.is_empty()
        && word.len() <= 64
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Whether `output` says `program` was not found, in one of the shells'
/// own formats. The program must be the word the shell names, not a
/// substring of it: `pnpm: command not found` does not say `npm` was
/// missing.
fn not_found(output: &str, program: &str) -> bool {
    let named = [
        // bash, and sh's `{program}: not found`
        format!("{program}: command not found"),
        format!("{program}: not found"),
        // zsh, fish
        format!("command not found: {program}"),
        format!("Unknown command: {program}"),
        // cmd, PowerShell
        format!("'{program}' is not recognized as"),
        format!("The term '{program}' is not recognized"),
        // python -m
        format!("No module named {program}"),
        format!("No module named '{program}'"),
    ];
    let name_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
    output.lines().any(|line| {
        named.iter().any(|pattern| {
            line.match_indices(pattern.as_str()).any(|(at, matched)| {
                // Whole words only at both ends of the match, so `pnpm`
                // never reads as `npm` and `pytest` never as `py`.
                let before = line[..at].chars().next_back();
                let after = line[at + matched.len()..].chars().next();
                !before.is_some_and(name_char) && !after.is_some_and(name_char)
            })
        })
    })
}

/// One fact: run `program` as `prefix program` here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    pub program: String,
    pub prefix: String,
    /// Digests of the sessions that confirmed it, most recent last.
    pub sessions: Vec<String>,
    /// Sessions since the last confirmation that ran the bare program fine.
    pub contradictions: u32,
}

impl Fact {
    pub fn told(&self) -> bool {
        self.sessions.len() >= CONFIRMATIONS
    }
}

/// The facts of one folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactBook {
    pub facts: Vec<Fact>,
}

impl FactBook {
    /// Fold one finished session, named by `session` (a digest).
    pub fn fold_session(&mut self, session: &str, observations: &Observations) {
        let mut confirmed = Vec::new();
        for (program, prefix) in &observations.repairs {
            match self
                .facts
                .iter_mut()
                .find(|fact| &fact.program == program && &fact.prefix == prefix)
            {
                Some(fact) => {
                    if !fact.sessions.iter().any(|seen| seen == session) {
                        fact.sessions.push(session.to_owned());
                        if fact.sessions.len() > CONFIRMATIONS * 2 {
                            fact.sessions.remove(0);
                        }
                    }
                    fact.contradictions = 0;
                    confirmed.push((program.clone(), prefix.clone()));
                }
                None => self.facts.push(Fact {
                    program: program.clone(),
                    prefix: prefix.clone(),
                    sessions: vec![session.to_owned()],
                    contradictions: 0,
                }),
            }
        }
        for program in &observations.bare_worked {
            if observations.missing.contains(program) {
                continue;
            }
            for fact in self
                .facts
                .iter_mut()
                .filter(|fact| &fact.program == program)
            {
                fact.contradictions += 1;
            }
        }
        // The facts are kept in order of their latest confirmation, oldest
        // first: a fact confirmed again moves to the end, as a new one is
        // pushed there.
        for key in confirmed {
            if let Some(at) = self
                .facts
                .iter()
                .position(|fact| fact.program == key.0 && fact.prefix == key.1)
            {
                let fact = self.facts.remove(at);
                self.facts.push(fact);
            }
        }
        self.facts
            .retain(|fact| fact.contradictions < CONTRADICTIONS);
        // Over the cap, the oldest untold fact goes first, then the oldest
        // told one, so a folder full of one-off repairs still learns new
        // ones.
        while self.facts.len() > MAX_KEPT {
            let at = self.facts.iter().position(|fact| !fact.told()).unwrap_or(0);
            self.facts.remove(at);
        }
    }

    /// The facts told at a session's start.
    pub fn told(&self) -> impl Iterator<Item = &Fact> {
        self.facts.iter().filter(|fact| fact.told()).take(MAX_TOLD)
    }

    /// The context a session starts with, or none when no fact is told.
    pub fn render(&self) -> Option<String> {
        let lines: Vec<String> = self
            .told()
            .map(|fact| {
                format!(
                    "- Run `{program}` as `{prefix} {program}` here; plain `{program}` is not \
                     found.",
                    program = fact.program,
                    prefix = fact.prefix
                )
            })
            .collect();
        if lines.is_empty() {
            return None;
        }
        Some(format!(
            "Learned from earlier sessions in this folder (Trace Commons):\n{}",
            lines.join("\n")
        ))
    }
}
