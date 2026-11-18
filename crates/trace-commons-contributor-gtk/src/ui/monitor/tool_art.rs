//! A tool's mark: its logo in its tint, or, for a tool with no logo, its
//! two-letter initials in its tint.
//!
//! A port of `GlassTool` in `macos/Sources/TCDesign/Components/Indicators.swift`:
//! which tools have artwork (`logo`), which tint each is drawn in (`tint`,
//! the glass tokens `toolClaude` and so on) and the two letters a tool
//! without artwork shows (`initials`). The artwork is the Mac's own path
//! data (`GlassLogoPaths.swift`, exported to `brand_tokens::logo`); Gemini
//! CLI and Cline have none in either shell.

use super::glyph::Artwork;
use super::traces_tree::SourceKind;

/// The colour a tool's mark is drawn in: one of the tool tints, or the
/// primary text colour for a tool with none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tint {
    Claude,
    Codex,
    Antigravity,
    OpenCode,
    Theia,
    Primary,
}

/// A tool's mark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolArt {
    pub logo: Option<Artwork>,
    pub tint: Tint,
    pub initials: &'static str,
}

fn logo(name: &str) -> Option<Artwork> {
    Artwork::TOOLS
        .iter()
        .find_map(|(known, artwork)| (*known == name).then_some(*artwork))
}

/// The mark the Mac draws for `kind` (`SourceKind.glassTool`).
pub fn tool_art(kind: SourceKind) -> ToolArt {
    let (logo, tint, initials) = match kind {
        SourceKind::ClaudeCode => (logo("claude"), Tint::Claude, "CC"),
        SourceKind::Codex => (logo("codex"), Tint::Codex, "Cx"),
        SourceKind::GeminiCli => (None, Tint::Antigravity, "Gm"),
        SourceKind::Cline => (None, Tint::Primary, "Cl"),
        SourceKind::Opencode => (logo("openCode"), Tint::OpenCode, "OC"),
    };
    ToolArt {
        logo,
        tint,
        initials,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_tool_has_the_macs_logo_tint_and_initials() {
        let art = |kind| tool_art(kind);
        assert_eq!(art(SourceKind::ClaudeCode).logo, logo("claude"));
        assert!(art(SourceKind::ClaudeCode).logo.is_some());
        assert_eq!(art(SourceKind::ClaudeCode).tint, Tint::Claude);
        assert_eq!(art(SourceKind::ClaudeCode).initials, "CC");
        assert_eq!(art(SourceKind::Codex).logo, logo("codex"));
        assert_eq!(art(SourceKind::Codex).tint, Tint::Codex);
        assert_eq!(art(SourceKind::Codex).initials, "Cx");
        // Gemini CLI has no artwork in either shell: its initials, in the
        // Antigravity tint.
        assert_eq!(art(SourceKind::GeminiCli).logo, None);
        assert_eq!(art(SourceKind::GeminiCli).tint, Tint::Antigravity);
        assert_eq!(art(SourceKind::GeminiCli).initials, "Gm");
        assert_eq!(art(SourceKind::Cline).logo, None);
        assert_eq!(art(SourceKind::Cline).tint, Tint::Primary);
        assert_eq!(art(SourceKind::Cline).initials, "Cl");
        assert_eq!(art(SourceKind::Opencode).logo, logo("openCode"));
        assert!(art(SourceKind::Opencode).logo.is_some());
        assert_eq!(art(SourceKind::Opencode).tint, Tint::OpenCode);
        assert_eq!(art(SourceKind::Opencode).initials, "OC");
    }
}
