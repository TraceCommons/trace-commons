//! Subscription plan limits — only ever the ones a user declared.
//!
//! The figures are the usage monitor's `core/plans.py` table, which was
//! reverse-engineered from observed sessions rather than published by
//! Anthropic. That is exactly the kind of number `AGENTS.md` rule 2 forbids
//! IronWire from asserting on its own.
//!
//! So it is not asserted. There is no default plan and nothing here is
//! consulted unless the user has written `plan = "max5"` in their config —
//! at which point the limit is *their* claim about *their* subscription, and
//! IronWire is doing arithmetic with a number it was given. The distinction
//! survives all the way to the screen, which says where the figure came from.
//!
//! Everything else uses [`crate::p90`], which needs no table at all.

use serde::{Deserialize, Serialize};

/// A plan the user can declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Plan {
    /// Claude Pro.
    Pro,
    /// Claude Max, 5×.
    Max5,
    /// Claude Max, 20×.
    Max20,
    /// Team. The monitor marks its figures unverified, and so do we.
    Team,
}

/// One plan's declared ceilings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanLimits {
    /// What to call it on screen.
    pub display_name: &'static str,
    /// Tokens per five-hour window.
    pub tokens: i64,
    /// USD per window at metered rates.
    pub cost_usd: f64,
    /// Messages per window.
    pub messages: i64,
    /// Whether even the source of these figures calls them a guess. Shown, so
    /// a user is never told a made-up ceiling with a straight face.
    pub unverified: bool,
}

impl Plan {
    /// The declared ceilings for this plan.
    #[must_use]
    pub const fn limits(self) -> PlanLimits {
        match self {
            Self::Pro => PlanLimits {
                display_name: "Pro",
                tokens: 19_000,
                cost_usd: 18.0,
                messages: 250,
                unverified: false,
            },
            Self::Max5 => PlanLimits {
                display_name: "Max 5×",
                tokens: 88_000,
                cost_usd: 35.0,
                messages: 1_000,
                unverified: false,
            },
            Self::Max20 => PlanLimits {
                display_name: "Max 20×",
                tokens: 220_000,
                cost_usd: 140.0,
                messages: 2_000,
                unverified: false,
            },
            Self::Team => PlanLimits {
                display_name: "Team",
                tokens: 19_000,
                cost_usd: 18.0,
                messages: 250,
                unverified: true,
            },
        }
    }

    /// Parse a config value. Case-insensitive; `max_5` and `max-5` are the
    /// same thing as `max5`, because a config file is written by hand.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_lowercase().replace(['-', '_', ' '], "").as_str() {
            "pro" => Some(Self::Pro),
            "max5" => Some(Self::Max5),
            "max20" => Some(Self::Max20),
            "team" => Some(Self::Team),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_is_parsed_however_it_was_typed() {
        assert_eq!(Plan::parse("Max5"), Some(Plan::Max5));
        assert_eq!(Plan::parse("max-5"), Some(Plan::Max5));
        assert_eq!(Plan::parse("MAX_20"), Some(Plan::Max20));
        assert_eq!(Plan::parse("pro"), Some(Plan::Pro));
    }

    #[test]
    fn an_unknown_plan_is_rejected_rather_than_guessed_at() {
        // Falling back to a default here would put a limit on screen that the
        // user never declared, which is the whole thing this module avoids.
        assert_eq!(Plan::parse("max"), None);
        assert_eq!(Plan::parse(""), None);
        assert_eq!(Plan::parse("enterprise"), None);
    }

    #[test]
    fn the_plans_whose_figures_are_a_guess_say_so() {
        assert!(Plan::Team.limits().unverified);
        assert!(!Plan::Max5.limits().unverified);
    }
}
