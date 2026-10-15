//! Where "Install {tool}" leads: each coding tool's own setup guide, over
//! https. The one table every surface reads -- first run's tool rows and the
//! managed-session `tools` rows alike (#1419 review, 2026-10-15).
//!
//! Keys are the discovery source slugs (`SourceKind` in the shells).
//! Sources, checked 2026-10-15:
//! - Claude Code: <https://code.claude.com/docs/en/setup>
//! - Codex: <https://developers.openai.com/codex/cli/>
//! - Gemini CLI: the installation guide the `google-gemini/gemini-cli`
//!   README links to.
//! - Cline: "Installing Cline" on docs.cline.bot, the docs site the
//!   `cline/cline` README links to.
//! - OpenCode: <https://opencode.ai/docs/>

/// `(source slug, setup guide)`, in the order first run asks about tools.
pub const INSTALL_GUIDES: [(&str, &str); 5] = [
    ("claude-code", "https://code.claude.com/docs/en/setup"),
    ("codex", "https://developers.openai.com/codex/cli/"),
    (
        "gemini-cli",
        "https://www.geminicli.com/docs/get-started/installation",
    ),
    (
        "cline",
        "https://docs.cline.bot/getting-started/installing-cline",
    ),
    ("opencode", "https://opencode.ai/docs/"),
];

/// The setup guide for a discovery source slug.
pub fn install_guide(source: &str) -> Option<&'static str> {
    INSTALL_GUIDES
        .iter()
        .find(|(slug, _)| *slug == source)
        .map(|(_, url)| *url)
}

/// The table as a JSON object, `{source slug: url}`.
pub fn json() -> serde_json::Value {
    INSTALL_GUIDES
        .iter()
        .map(|(slug, url)| ((*slug).to_owned(), serde_json::Value::from(*url)))
        .collect::<serde_json::Map<_, _>>()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_guide_is_https_and_every_asked_tool_has_one() {
        for slug in ["claude-code", "codex", "gemini-cli", "cline", "opencode"] {
            let url = install_guide(slug).unwrap_or_else(|| panic!("{slug}"));
            assert!(url.starts_with("https://"), "{url}");
        }
        assert_eq!(json().as_object().unwrap().len(), INSTALL_GUIDES.len());
    }

    /// The managed `tools` rows read this table, not a copy of it.
    #[test]
    fn managed_tool_rows_read_the_same_table() {
        let rows = crate::managed::profiles::tools();
        for (row, slug) in rows
            .as_array()
            .unwrap()
            .iter()
            .zip(["claude-code", "codex", "opencode"])
        {
            assert_eq!(row["install_url"], install_guide(slug).unwrap());
        }
    }

    /// First run no longer carries its own install links.
    #[test]
    fn first_run_copy_has_no_install_table() {
        let copy = serde_json::to_value(crate::first_run_copy::first_run_copy()).unwrap();
        assert!(copy["folders"].get("install_urls").is_none());
    }
}
