//! Rendering the daemon's state for a terminal.
//!
//! One rule governs every number here: if the provider did not tell us, we say
//! `unknown` (`docs/CRITIQUE.md` §4). A plausible fabricated percentage costs
//! us belief in the numbers that *are* real, which is most of the value of this
//! screen.
//!
//! The session section is the one place that shows a figure the provider did
//! not state, and it is not an exception to that rule: it measures IronWire's
//! own traffic and labels every derived number with where it came from. See
//! [`ironwire_usage`] for why that is a different claim from quota.

use ironwire_ledger::{Exchange, Summary};
use ironwire_proxy::control::{
    BackendView, BalanceView, HeadroomView, HealthView, LogView, StatusView,
};
use ironwire_update::UpdateStatus;
use ironwire_usage::{Basis, SessionUsage, UsageReport};

use crate::style::Style;

/// Render `ironwire log`.
#[must_use]
pub(crate) fn log(view: &LogView, style: Style) -> String {
    let mut out = String::new();
    if !view.enabled {
        out.push_str("Local trace capture is off.\n\n");
        out.push_str(&format!(
            "  Turn it on with {} in\n",
            style.action("capture.enabled = true")
        ));
        out.push_str("  $IRONWIRE_HOME/config.toml, then restart the daemon.\n");
        return out;
    }
    if view.exchanges.is_empty() {
        out.push_str("No exchanges recorded yet.\n");
        return out;
    }

    out.push_str(&style.dim(format!(
        "{:<21} {:<BACKEND_WIDTH$} {:<18} {:>9} {:>8} {:>7}\n",
        "when", "backend", "model", "in/cached", "out", "took"
    )));
    for exchange in &view.exchanges {
        out.push_str(&exchange_row(exchange, style));
    }
    out.push('\n');
    out.push_str(&summary_block(&view.last_24h, style));
    out
}

/// Wide enough for a login's id. Ids like `claude-work` and `claude-personal`
/// share their first dozen characters, and cut at fifteen they stop being
/// told apart in exactly the table that says which login paid.
const BACKEND_WIDTH: usize = 20;

fn exchange_row(exchange: &Exchange, style: Style) -> String {
    let model = exchange
        .served_model
        .as_deref()
        .or(exchange.requested_model.as_deref())
        .unwrap_or("—");
    // An exchange whose usage the provider never reported shows a dash, not a
    // zero. A fabricated zero would understate what the user actually spent.
    let tokens = |value: Option<i64>| value.map_or_else(|| "—".to_string(), compact);
    let cached = exchange
        .cache_read_tokens
        .map_or_else(String::new, |v| format!("/{}", compact(v)));
    // Only when there is one, so the common warm row does not get noisier.
    let written = exchange
        .cache_write_tokens
        .filter(|v| *v > 0)
        .map_or_else(String::new, |v| format!("+{}", compact(v)));
    let took = exchange.total_ms.map_or_else(
        || "—".to_string(),
        |ms| format!("{:.1}s", ms as f64 / 1000.0),
    );
    let status = if exchange.status == 200 {
        String::new()
    } else {
        style.bad(format!("  [{}]", exchange.status))
    };
    // Shown only when the filter was on. Zero substitutions on a turn the user
    // believed was being filtered is exactly the signal they need, so it is
    // printed rather than suppressed as uninteresting (`docs/PRIVACY.md` §7).
    let filtered = exchange
        .substitutions
        .map_or_else(String::new, |n| style.dim(format!("  🔒{n}")));

    format!(
        "{:<21} {:<BACKEND_WIDTH$} {:<18} {:>9} {:>8} {:>7}{status}{filtered}\n",
        style.dim(exchange.started_at.format("%Y-%m-%d %H:%M:%S")),
        style.name(format!(
            "{:<BACKEND_WIDTH$}",
            truncate(&exchange.backend, BACKEND_WIDTH)
        )),
        truncate(model, 18),
        format!("{}{cached}{written}", tokens(exchange.input_tokens)),
        tokens(exchange.output_tokens),
        took,
    )
}

fn summary_block(summary: &Summary, style: Style) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} {} exchanges",
        style.dim("last 24h:"),
        style.value(summary.exchanges)
    ));
    if summary.without_usage > 0 {
        out.push_str(&format!(
            " ({} with no usage reported)",
            summary.without_usage
        ));
    }
    out.push('\n');
    // Writes named separately because they are billed above base input rate:
    // a breakdown that folds them into "in" cannot explain a bill that moved.
    let written = if summary.cache_write_tokens > 0 {
        format!(" · {} written", compact(summary.cache_write_tokens))
    } else {
        String::new()
    };
    out.push_str(&format!(
        "  {} {} in · {} cached{written} · {} out\n",
        style.dim("tokens:"),
        compact(summary.input_tokens),
        compact(summary.cache_read_tokens),
        compact(summary.output_tokens),
    ));
    // Omitted entirely rather than shown as "unknown" when nothing reported:
    // absence is quieter and equally honest.
    if let Some(rate) = summary.cache_hit_rate {
        let cold = if summary.cold_starts > 0 {
            format!(" · {} cold starts", style.value(summary.cold_starts))
        } else {
            String::new()
        };
        out.push_str(&format!(
            "  {} {:.0}% hit rate{cold}\n",
            style.dim("cache: "),
            rate * 100.0
        ));
    }
    if !summary.by_backend.is_empty() {
        let split = summary
            .by_backend
            .iter()
            .map(|(backend, count)| format!("{} {count}", style.name(backend)))
            .collect::<Vec<_>>()
            .join(" · ");
        out.push_str(&format!("  {} {split}\n", style.dim("routed:")));
    }
    out
}

fn compact(value: i64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

/// The same scale for a rate, which is rarely a whole number.
fn compact_rate(value: f64) -> String {
    if value >= 1_000_000.0 {
        format!("{:.1}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.1}k", value / 1_000.0)
    } else if value >= 10.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    text.chars()
        .take(width.saturating_sub(1))
        .collect::<String>()
        + "…"
}

/// Render the full status screen.
#[must_use]
pub(crate) fn status(status: &StatusView, style: Style) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} {} — {}\n\n",
        style.heading("IronWire"),
        status.version,
        style.action(format!("http://127.0.0.1:{}", status.port))
    ));

    if status.backends.is_empty() {
        out.push_str("No backends configured.\n\n");
        out.push_str(&format!(
            "  Run {}, or set\n",
            style.action("`ironwire connect claude --subscription`")
        ));
        out.push_str("  ANTHROPIC_API_KEY and restart the daemon.\n");
        return out;
    }

    for backend in &status.backends {
        out.push_str(&backend_block(backend, style));
        out.push('\n');
    }

    let balance = balance_block(&status.balance, style);
    if !balance.is_empty() {
        out.push_str(&balance);
        out.push('\n');
    }

    let usage = usage_block(&status.usage, style);
    if !usage.is_empty() {
        out.push_str(&usage);
        out.push('\n');
    }

    if let Some(pin) = &status.pin {
        out.push_str(&format!(
            "Pinned to {} (clear with {})\n",
            style.name(pin),
            style.action("`ironwire pin`")
        ));
    }
    out.push_str(&format!(
        "{} conversation(s) with a sticky route\n",
        status.tracked_conversations
    ));
    // Permanent, not a startup message that scrolls away: with the filter on,
    // IronWire is mutating requests, and that must never become invisible
    // (`docs/PRIVACY.md` §3). It states what is running, never what the user is
    // protected from.
    if let Some(privacy) = &status.privacy {
        out.push_str(&format!("privacy filter: {privacy}\n"));
    }
    // Permanent for the same reason: it is a risk the user chose, the provider
    // can see it happen, and a setting like that must not drop out of sight
    // between the day it was turned on and the day it matters.
    if status.cross_account_fallback {
        out.push_str(&format!(
            "{} — a client whose own subscription is spent may be served by another \
             of yours (docs/TRUST.md)\n",
            style.warn("cross-account fallback: on")
        ));
    }
    if status.catalog_serial > 0 {
        out.push_str(&format!(
            "provider catalog: serial {}\n",
            status.catalog_serial
        ));
    }
    out.push_str(&update_line(&status.update, style));
    out
}

/// Notify-only: say a newer release exists and how to get it. Never act.
fn update_line(update: &UpdateStatus, style: Style) -> String {
    match update {
        UpdateStatus::Available {
            latest,
            upgrade_command,
            ..
        } => match upgrade_command {
            Some(command) => format!(
                "\n{}\n",
                style.warn(format!("ironwire {latest} is available — {command}"))
            ),
            None => format!(
                "\n{}\n",
                style.warn(format!("ironwire {latest} is available"))
            ),
        },
        UpdateStatus::Unsupported {
            latest,
            minimum_supported,
            upgrade_command,
        } => {
            let how = upgrade_command
                .as_deref()
                .map_or_else(String::new, |c| format!(" — {c}"));
            format!(
                "\n{}\n",
                style.bad(format!(
                    "This build is below the supported floor ({minimum_supported}); providers may \n\
                     have changed in ways it does not handle. {latest} is available{how}"
                ))
            )
        }
        UpdateStatus::UpToDate | UpdateStatus::Unknown => String::new(),
    }
}

fn backend_block(backend: &BackendView, style: Style) -> String {
    let mut out = String::new();
    out.push_str(&format!("{}\n", style.name(&backend.name)));

    if !backend.authenticated {
        let detail = backend.detail.as_deref().unwrap_or("not authenticated");
        out.push_str(&format!("  {} — {detail}\n", style.bad("not connected")));
        return out;
    }
    if !backend.consented {
        out.push_str(&format!(
            "  {} — run {}\n",
            style.warn("awaiting consent"),
            style.action(format!("`{}`", consent_command(backend)))
        ));
        return out;
    }

    out.push_str(&format!(
        "  {} · {}\n",
        style.good("connected"),
        backend.kind.replace('_', " ")
    ));
    out.push_str(&format!(
        "  {} {}\n",
        style.dim("capacity:"),
        headroom(&backend.headroom, style)
    ));
    // Only when it is not the boring answer. A "circuit: closed" line on every
    // healthy backend is noise that trains people to stop reading the block.
    if let Some(line) = health_line(&backend.health, style) {
        out.push_str(&format!("  {line}\n"));
    }
    if !backend.models.is_empty() {
        // Best-first, so the head of the list is the part worth reading. A
        // general endpoint can offer fifty models, and printing all of them
        // turns the one screen a user checks under pressure into a wall of
        // text — the count still says how many there are.
        const SHOWN: usize = 6;
        let shown = backend
            .models
            .iter()
            .take(SHOWN)
            .cloned()
            .collect::<Vec<_>>();
        let rest = backend.models.len().saturating_sub(shown.len());
        let more = if rest > 0 {
            style.dim(format!(" (+{rest} more)"))
        } else {
            String::new()
        };
        out.push_str(&format!(
            "  {} {}{more}\n",
            style.dim("models:"),
            shown.join(", ")
        ));
    }
    out
}

/// What to run to consent to this backend's subscription.
///
/// From the daemon's `connect_command`, which knows which login a backend
/// reads: with two Claude logins, a command written here from the product
/// alone would enable the wrong one. An older daemon sends none, and had only
/// the default logins, which the id still names.
pub(crate) fn consent_command(backend: &BackendView) -> String {
    let connect = backend.connect_command.clone().unwrap_or_else(|| {
        match backend.id.as_str() {
            "codex-sub" => "ironwire connect codex",
            _ => "ironwire connect claude",
        }
        .to_string()
    });
    format!("{connect} --subscription")
}

/// Say a backend is being skipped, and why — a silently-skipped backend looks
/// like one that is simply not being chosen.
fn health_line(health: &HealthView, style: Style) -> Option<String> {
    match health.circuit.as_str() {
        "open" => Some(style.bad(match health.retry_in_secs {
            Some(secs) if secs > 0 => format!(
                "skipping after {} consecutive failures · next try in {}",
                health.consecutive_failures,
                duration(secs)
            ),
            _ => format!(
                "skipping after {} consecutive failures",
                health.consecutive_failures
            ),
        })),
        "halfopen" | "half_open" => Some(style.warn("recovering — trying it again now")),
        _ if health.consecutive_failures > 0 => Some(style.warn(format!(
            "{} recent failure(s), still in use",
            health.consecutive_failures
        ))),
        _ => None,
    }
}

/// Every pool as one balance — see `BalanceView` for why this counts pools
/// rather than merging them into a single number.
fn balance_block(balance: &BalanceView, style: Style) -> String {
    let mut parts = Vec::new();
    if balance.available > 0 {
        parts.push(style.good(match balance.free_available {
            0 => format!("{} pool(s) available", balance.available),
            free if free == balance.available => {
                format!("{} pool(s) available, all already paid for", free)
            }
            free => format!(
                "{} pool(s) available ({free} already paid for)",
                balance.available
            ),
        }));
    }
    if balance.unknown > 0 {
        // Not folded into "available": the provider has told us nothing, and
        // reporting that as headroom is exactly the fabrication this screen
        // exists to avoid.
        parts.push(style.dim(format!("{} not yet reporting", balance.unknown)));
    }
    if balance.unavailable > 0 {
        parts.push(style.bad(match balance.next_available_at {
            Some(at) => {
                let secs = (at - chrono::Utc::now()).num_seconds().max(0);
                format!(
                    "{} unavailable · first back in {}",
                    balance.unavailable,
                    duration(secs)
                )
            }
            None => format!("{} unavailable", balance.unavailable),
        }));
    }

    if parts.is_empty() {
        return String::new();
    }
    let mut out = format!("{} {}\n", style.heading("Balance:"), parts.join(" · "));

    // Two different currencies, so two lines rather than one number. A
    // subscription is spent in percent of a window you have already bought; an
    // API key is spent in dollars you have not. Collapsing them into one figure
    // is how "$0.16 of metered spend" came to describe a day that ran entirely
    // on subscriptions and was billed nothing.
    let used: Vec<String> = balance
        .subscription_used
        .iter()
        .filter(|s| s.exchanges > 0 || s.used_pct.is_some())
        .map(|s| match s.used_pct {
            Some(pct) => format!(
                "{} {}",
                style.name(&s.name),
                style.by_usage(f64::from(pct), format!("{pct:.0}% used"))
            ),
            None => format!("{} {}", style.name(&s.name), style.dim("not yet reported")),
        })
        .collect();
    if !used.is_empty() {
        out.push_str(&format!(
            "  {} {}\n",
            style.dim("subscriptions:"),
            used.join(" · ")
        ));
    }
    if let Some(rate) = balance.cache_hit_rate {
        out.push_str(&format!(
            "  cache: {:.0}% hit rate over {} exchange{} today\n",
            rate * 100.0,
            balance.cache_exchanges,
            if balance.cache_exchanges == 1 {
                ""
            } else {
                "s"
            }
        ));
    }
    // Permanent, like the privacy-filter line: a limit the user cannot see is
    // one they cannot trust, and this is the number they set it against.
    if let Some(cap) = &balance.spend_cap {
        out.push_str(&format!(
            "  spend today: ${:.2} of ${:.2} cap\n",
            cap.spent_usd + 0.0,
            cap.cap_usd
        ));
    }
    match balance.spend_today_usd {
        // Zero is a result, not an absence: it is the sentence "nothing was
        // billed today", which is the whole point of routing to a subscription.
        // `+ 0.0` because summing no dollars at all gives `-0.0` in IEEE 754,
        // and "$-0.00" reads like a refund.
        Some(spend) => out.push_str(&format!(
            "  {} {}\n",
            style.dim("metered spend today:"),
            style.value(format!("${:.2}", spend + 0.0))
        )),
        None => out.push_str(&format!(
            "  {} {}\n",
            style.dim("metered spend today:"),
            style.dim("not recorded (ledger off)")
        )),
    }
    out
}

fn headroom(headroom: &HeadroomView, style: Style) -> String {
    match headroom {
        HeadroomView::Observed {
            used_pct,
            observed_secs_ago,
            resets_in_secs,
        } => {
            let pct = f64::from(*used_pct);
            let bar = meter(pct, style);
            let age = style.dim(format!("observed {} ago", duration(*observed_secs_ago)));
            let used = style.by_usage(pct, format!("{used_pct:.0}% used"));
            match resets_in_secs {
                Some(reset) if *reset > 0 => format!(
                    "{bar} {used} · resets in {} · {age}",
                    style.value(duration(*reset))
                ),
                _ => format!("{bar} {used} · {age}"),
            }
        }
        HeadroomView::Exhausted { retry_in_secs } => {
            style.bad(format!("exhausted · retry in {}", duration(*retry_in_secs)))
        }
        // Said differently from `exhausted` on purpose: the provider would have
        // served this. The user is the one who said stop, so the line names
        // their own number rather than a provider's window.
        HeadroomView::CapReached {
            spent_usd,
            cap_usd,
            resets_in_secs,
        } => style.bad(format!(
            "cap reached — ${spent_usd:.2} of ${cap_usd:.2} · resets in {}",
            duration(*resets_in_secs)
        )),
        // Not "0%", not "healthy" — we genuinely do not know, and saying so is
        // what makes the other rows worth believing.
        HeadroomView::Unknown => style.dim("unknown (the provider has not reported yet)"),
    }
}

/// The session section: how fast this is going, measured from our own ledger.
///
/// Kept visibly separate from `capacity:` above, and worded so the difference
/// cannot be missed. That line is the provider's word; this one is ours, and a
/// reader who conflates them ends up trusting an extrapolation the way they
/// would trust a rate-limit header.
fn usage_block(report: &UsageReport, style: Style) -> String {
    if report.sessions.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "{} {}\n",
        style.heading(format!("Session ({}h window)", report.session_hours)),
        style.dim("— measured from IronWire's own ledger, not reported by the provider")
    );
    for session in &report.sessions {
        out.push_str(&session_block(session, style));
    }
    out
}

fn session_block(session: &SessionUsage, style: Style) -> String {
    let mut out = format!(
        "  {} {}\n",
        style.name(&session.backend),
        style.dim(format!(
            "opened {} ago · closes in {}",
            duration_minutes(session.elapsed_minutes),
            duration_minutes(session.remaining_minutes)
        ))
    );

    // The bar exists only when there is something to be a fraction *of*.
    // Drawing an empty one against no ceiling would imply a limit we do not
    // have.
    if let (Some(ceiling), Some(pct)) = (session.ceiling.as_ref(), session.used_pct) {
        let caveat = if ceiling.unverified {
            style.dim(" (unverified figures)")
        } else {
            String::new()
        };
        out.push_str(&format!(
            "    {} {} of {}{caveat}\n",
            meter(pct, style),
            style.by_usage(pct, format!("{pct:.0}%")),
            ceiling.description
        ));
    }

    let mut used = format!(
        "{} tokens · {} exchange(s)",
        style.value(compact(session.tokens.total())),
        session.exchanges
    );
    if session.without_usage > 0 {
        // The total below is missing these, so it is an undercount and the
        // reader has to be told by how much.
        used.push_str(&style.dim(format!(
            " ({} with no usage reported)",
            session.without_usage
        )));
    }
    if session.cost_usd > 0.0 {
        used.push_str(&style.dim(format!(" · ${:.2} at metered rates", session.cost_usd)));
    }
    out.push_str(&format!("    {} {used}\n", style.dim("used:")));

    match session.burn {
        Some(burn) => {
            let mut line = format!(
                "{} {}",
                style.value(compact_rate(burn.tokens_per_minute)),
                style.dim("tokens/min")
            );
            if burn.cost_per_hour > 0.0 {
                line.push_str(&style.dim(format!(" · ${:.2}/hour", burn.cost_per_hour)));
            }
            if let Some(hourly) = session.hourly_tokens_per_minute {
                line.push_str(&style.dim(format!(" · {}/min last hour", compact_rate(hourly))));
            }
            out.push_str(&format!("    {} {line}\n", style.dim("burn:")));
        }
        // Not "0 tokens/min": that reads as "you have stopped", which is a
        // different claim from "one request is not an interval".
        None => out.push_str(&format!(
            "    {} {}\n",
            style.dim("burn:"),
            style.dim("not enough traffic in this window to measure a rate")
        )),
    }

    if let Some(projection) = session.projection {
        let mut line = format!(
            "{} tokens by the time it closes",
            style.value(compact(projection.total_tokens))
        );
        if projection.total_cost_usd > 0.0 {
            line.push_str(&style.dim(format!(" · ${:.2}", projection.total_cost_usd)));
        }
        out.push_str(&format!("    {} {line}\n", style.dim("at this rate:")));
    }

    // The one sentence this whole section exists to be able to say.
    if let Some(minutes) = session.exhausts_in_minutes {
        let ceiling = session.ceiling.as_ref().map_or("that ceiling", |c| {
            if c.basis == Basis::Declared {
                "your declared limit"
            } else {
                "your usual ceiling"
            }
        });
        if session.exhausts_before_close() {
            out.push_str(&format!(
                "    {}\n",
                style.bad(format!(
                    "you reach {ceiling} in {} — {} before the window closes",
                    duration_minutes(minutes),
                    duration_minutes(session.remaining_minutes - minutes)
                ))
            ));
        } else {
            out.push_str(&format!(
                "    {}\n",
                style.good(format!("the window closes before you reach {ceiling}"))
            ));
        }
    } else if session.ceiling.is_none() {
        out.push_str(&format!(
            "    {}\n",
            style.dim(
                "nothing to compare against yet — needs one completed session, \
                 or `usage.plan` in config.toml"
            )
        ));
    }
    out
}

fn meter(used_pct: f64, style: Style) -> String {
    const WIDTH: usize = 10;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is clamped to 0..=100 before the cast"
    )]
    let filled = ((used_pct.clamp(0.0, 100.0) / 100.0) * WIDTH as f64).round() as usize;
    let filled = filled.min(WIDTH);
    format!(
        "[{}{}]",
        style.by_usage(used_pct, "█".repeat(filled)),
        style.dim("░".repeat(WIDTH.saturating_sub(filled)))
    )
}

fn duration(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 60 {
        return format!("{secs}s");
    }
    if secs < 3600 {
        return format!("{}m", secs / 60);
    }
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    if minutes == 0 {
        format!("{hours}h")
    } else {
        format!("{hours}h{minutes}m")
    }
}

/// The same scale, from the minutes the usage estimates are computed in.
fn duration_minutes(minutes: f64) -> String {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "minutes come from a bounded window, far inside i64"
    )]
    let secs = (minutes.max(0.0) * 60.0) as i64;
    duration(secs)
}

#[cfg(test)]
mod log_tests {
    use super::*;
    use chrono::{DateTime, Utc};

    fn exchange() -> Exchange {
        Exchange {
            id: None,
            model_alias_resolved: None,
            started_at: DateTime::<Utc>::from_timestamp(1_700_000_000, 0).expect("timestamp"),
            ttfb_ms: Some(400),
            total_ms: Some(9_100),
            facade: "anthropic".into(),
            path: "/v1/messages".into(),
            conversation: "c".into(),
            client_session_id: None,
            backend: "claude-sub".into(),
            requested_model: Some("claude-opus-4-6".into()),
            served_model: Some("claude-opus-4-6".into()),
            upstream_id: None,
            request_sha256: None,
            response_sha256: None,
            body_ref: None,
            rung: "preferred".into(),
            attempts: 1,
            input_tokens: Some(12),
            cache_read_tokens: Some(98_000),
            cache_write_tokens: Some(2_048),
            output_tokens: Some(137),
            cost_usd: Some(0.42),
            substitutions: None,
            status: 200,
            error: None,
            confidence: None,
            proof: None,
        }
    }

    fn view(exchanges: Vec<Exchange>, last_24h: Summary) -> LogView {
        LogView {
            enabled: true,
            exchanges,
            last_24h,
        }
    }

    #[test]
    fn unreported_usage_renders_as_a_dash_not_a_zero() {
        // A fabricated zero understates what the user actually spent.
        let mut unknown = exchange();
        unknown.input_tokens = None;
        unknown.output_tokens = None;
        unknown.cache_read_tokens = None;
        let row = exchange_row(&unknown, Style::plain());
        assert!(row.contains('—'));
        assert!(
            !row.contains(" 0 "),
            "rendered a zero we never observed: {row}"
        );
    }

    #[test]
    fn a_row_shows_the_model_that_actually_served_it() {
        let mut substituted = exchange();
        substituted.requested_model = Some("claude-opus-4-6".into());
        substituted.served_model = Some("claude-sonnet-4-6".into());
        assert!(exchange_row(&substituted, Style::plain()).contains("claude-sonnet-4-6"));
    }

    #[test]
    fn a_non_200_exchange_shows_its_status() {
        let mut failed = exchange();
        failed.status = 429;
        assert!(exchange_row(&failed, Style::plain()).contains("[429]"));
        assert!(!exchange_row(&exchange(), Style::plain()).contains('['));
    }

    #[test]
    fn a_window_with_no_reported_usage_shows_no_cache_line() {
        // Absence rather than "cache: unknown": quieter, and equally honest.
        let rendered = summary_block(
            &Summary {
                exchanges: 3,
                without_usage: 3,
                cache_hit_rate: None,
                ..Summary::default()
            },
            Style::plain(),
        );
        assert!(!rendered.contains("hit rate"), "got: {rendered}");
        assert!(!rendered.contains("cold start"), "got: {rendered}");
        // The token line still lists what was summed; it is the *rate* that is
        // withheld, because a rate over nothing is a fabrication.
        assert!(rendered.contains("0 cached"), "got: {rendered}");
    }

    /// Distinct from the above and it must read that way: this cache really is
    /// doing nothing.
    #[test]
    fn an_all_cold_window_says_zero_percent() {
        let rendered = summary_block(
            &Summary {
                exchanges: 4,
                cache_write_tokens: 50_000,
                cache_hit_rate: Some(0.0),
                cold_starts: 4,
                ..Summary::default()
            },
            Style::plain(),
        );
        assert!(rendered.contains("0% hit rate"), "got: {rendered}");
        assert!(rendered.contains("4 cold starts"), "got: {rendered}");
        assert!(rendered.contains("50.0k written"), "got: {rendered}");
    }

    /// The common warm row must not get noisier for a number that is usually
    /// zero.
    #[test]
    fn a_row_shows_cache_writes_only_when_there_are_some() {
        let mut warm = exchange();
        warm.cache_write_tokens = Some(0);
        assert!(!exchange_row(&warm, Style::plain()).contains('+'));

        let mut cold = exchange();
        cold.cache_write_tokens = Some(2_048);
        assert!(exchange_row(&cold, Style::plain()).contains("+2.0k"));
    }

    #[test]
    fn the_summary_names_how_many_had_no_usage() {
        let rendered = summary_block(
            &Summary {
                exchanges: 10,
                without_usage: 3,
                input_tokens: 1_200,
                cache_read_tokens: 2_400_000,
                output_tokens: 900,
                cost_usd: 1.25,
                by_backend: vec![("claude-sub".into(), 7), ("anthropic-key".into(), 3)],
                cost_by_backend: vec![("claude-sub".into(), 0.0), ("anthropic-key".into(), 1.25)],
                cache_write_tokens: 4_096,
                cache_hit_rate: Some(0.97),
                cold_starts: 2,
                cache_by_backend: vec![("claude-sub".into(), 2_400_000, 4_096)],
            },
            Style::plain(),
        );
        assert!(rendered.contains("10 exchanges"));
        assert!(rendered.contains("3 with no usage reported"));
        assert!(rendered.contains("2.4M cached"));
        assert!(rendered.contains("claude-sub 7"));
    }

    #[test]
    fn capture_being_off_explains_how_to_turn_it_on() {
        let rendered = log(
            &LogView {
                enabled: false,
                exchanges: vec![],
                last_24h: Summary::default(),
            },
            Style::plain(),
        );
        assert!(rendered.contains("capture.enabled = true"));
    }

    #[test]
    fn an_empty_ledger_says_so_rather_than_printing_an_empty_table() {
        let rendered = log(&view(vec![], Summary::default()), Style::plain());
        assert!(rendered.contains("No exchanges recorded yet"));
    }

    #[test]
    fn compact_numbers_stay_readable() {
        assert_eq!(compact(42), "42");
        assert_eq!(compact(1_500), "1.5k");
        assert_eq!(compact(2_400_000), "2.4M");
        assert_eq!(compact_rate(1.25), "1.2");
        assert_eq!(compact_rate(42.4), "42");
        assert_eq!(compact_rate(1_500.0), "1.5k");
    }

    /// Two logins' ids share a long prefix. Cut short, they would be one
    /// backend in the table that says which login paid.
    #[test]
    fn login_ids_are_not_cut_into_each_other_in_the_log() {
        let mut personal = exchange();
        personal.backend = "claude-personal".into();
        let mut work = exchange();
        work.backend = "claude-personal-2".into();
        assert!(exchange_row(&personal, Style::plain()).contains("claude-personal "));
        assert!(exchange_row(&work, Style::plain()).contains("claude-personal-2 "));
    }

    #[test]
    fn long_names_are_truncated_without_panicking_on_multibyte_text() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("a-very-long-backend-name", 10), "a-very-lo…");
        assert_eq!(truncate("日本語のモデル名です", 5), "日本語の…");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(headroom: HeadroomView) -> BackendView {
        BackendView {
            id: "claude-sub".into(),
            name: "Claude subscription".into(),
            kind: "subscription".into(),
            authenticated: true,
            consented: true,
            detail: None,
            headroom,
            health: HealthView::default(),
            models: vec!["claude-opus-4-6".into()],
            connect_command: None,
        }
    }

    pub(super) fn status_view(backends: Vec<BackendView>) -> StatusView {
        StatusView {
            version: "0.1.0".into(),
            port: 8463,
            tracked_conversations: 0,
            pin: None,
            backends,
            balance: BalanceView::default(),
            privacy: None,
            catalog_serial: 0,
            update: UpdateStatus::Unknown,
            last_route: None,
            usage: UsageReport::default(),
            cross_account_fallback: false,
        }
    }

    #[test]
    fn unknown_capacity_says_unknown_and_shows_no_number() {
        let rendered = backend_block(&view(HeadroomView::Unknown), Style::plain());
        assert!(rendered.contains("unknown"));
        assert!(!rendered.contains('%'), "we must not imply a measurement");
        assert!(!rendered.contains('█'));
    }

    #[test]
    fn an_observation_is_shown_with_its_age() {
        let rendered = backend_block(
            &view(HeadroomView::Observed {
                used_pct: 82.0,
                observed_secs_ago: 40,
                resets_in_secs: Some(8040),
            }),
            Style::plain(),
        );
        assert!(rendered.contains("82% used"));
        assert!(rendered.contains("observed 40s ago"));
        assert!(rendered.contains("resets in 2h14m"));
    }

    #[test]
    fn an_unauthenticated_backend_explains_itself_and_shows_no_capacity() {
        let mut backend = view(HeadroomView::Unknown);
        backend.authenticated = false;
        backend.detail = Some("Claude Code is not logged in on this machine".into());
        let rendered = backend_block(&backend, Style::plain());
        assert!(rendered.contains("not connected"));
        assert!(rendered.contains("not logged in"));
        assert!(!rendered.contains("capacity"));
    }

    #[test]
    fn an_unconsented_subscription_points_at_the_command_that_fixes_it() {
        let mut backend = view(HeadroomView::Unknown);
        backend.consented = false;
        assert!(
            backend_block(&backend, Style::plain())
                .contains("ironwire connect claude --subscription")
        );
    }

    /// Two logins of one product are two backends, and the screen has to let
    /// a person tell them apart and enable the right one. A hint composed
    /// from the product alone would point both at the default login.
    #[test]
    fn two_logins_render_as_two_backends_each_with_its_own_consent_command() {
        let mut personal = view(HeadroomView::Unknown);
        personal.consented = false;
        personal.connect_command = Some("ironwire connect claude".into());
        let mut work = view(HeadroomView::Unknown);
        work.id = "claude-work".into();
        work.name = "Claude subscription (~/.claude-work)".into();
        work.consented = false;
        work.connect_command = Some("ironwire connect claude --config-dir ~/.claude-work".into());

        let rendered = status(&status_view(vec![personal, work]), Style::plain());
        assert!(
            rendered.contains("Claude subscription\n"),
            "the default login lost its heading: {rendered}"
        );
        assert!(rendered.contains("Claude subscription (~/.claude-work)\n"));
        assert!(rendered.contains("run `ironwire connect claude --subscription`"));
        assert!(
            rendered.contains(
                "run `ironwire connect claude --config-dir ~/.claude-work --subscription`"
            )
        );
    }

    /// The hint used to name Claude for every backend, Codex included.
    #[test]
    fn an_unconsented_codex_login_is_not_pointed_at_claude() {
        let mut backend = view(HeadroomView::Unknown);
        backend.id = "codex-sub".into();
        backend.consented = false;
        assert!(
            backend_block(&backend, Style::plain())
                .contains("`ironwire connect codex --subscription`")
        );
        backend.connect_command = Some("ironwire connect codex --config-dir ~/.codex-work".into());
        assert!(
            backend_block(&backend, Style::plain())
                .contains("`ironwire connect codex --config-dir ~/.codex-work --subscription`")
        );
    }

    /// A risk the user chose stays on the screen for as long as it is on.
    #[test]
    fn cross_account_fallback_is_said_whenever_it_is_on_and_never_otherwise() {
        let mut view = status_view(vec![view(HeadroomView::Unknown)]);
        assert!(!status(&view, Style::plain()).contains("cross-account"));
        view.cross_account_fallback = true;
        let rendered = status(&view, Style::plain());
        assert!(
            rendered.contains("cross-account fallback: on"),
            "{rendered}"
        );
    }

    #[test]
    fn the_meter_tracks_the_percentage() {
        let plain = Style::plain();
        assert_eq!(meter(0.0, plain), "[░░░░░░░░░░]");
        assert_eq!(meter(100.0, plain), "[██████████]");
        assert_eq!(meter(50.0, plain), "[█████░░░░░]");
        // Out-of-range input must not panic or overflow the bar.
        assert_eq!(meter(-5.0, plain), "[░░░░░░░░░░]");
        assert_eq!(meter(150.0, plain), "[██████████]");
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration(45), "45s");
        assert_eq!(duration(600), "10m");
        assert_eq!(duration(7200), "2h");
        assert_eq!(duration(8040), "2h14m");
        assert_eq!(duration(-10), "0s");
        assert_eq!(duration_minutes(134.0), "2h14m");
        assert_eq!(duration_minutes(-3.0), "0s");
    }

    #[test]
    fn an_empty_daemon_tells_you_what_to_run() {
        let rendered = status(&status_view(vec![]), Style::plain());
        assert!(rendered.contains("ironwire connect claude"));
    }

    #[test]
    fn a_healthy_backend_says_nothing_about_its_circuit() {
        // A "circuit: closed" line on every healthy backend is noise that
        // trains people to stop reading the block.
        let rendered = backend_block(&view(HeadroomView::Unknown), Style::plain());
        assert!(!rendered.contains("circuit"));
        assert!(!rendered.contains("skipping"));
    }

    #[test]
    fn a_backend_being_skipped_says_so_and_says_when_it_comes_back() {
        // Otherwise a skipped backend is indistinguishable from one that is
        // simply not being chosen, and the user has no idea why their requests
        // are landing somewhere more expensive.
        let mut backend = view(HeadroomView::Unknown);
        backend.health = HealthView {
            circuit: "open".into(),
            consecutive_failures: 5,
            retry_in_secs: Some(90),
        };
        let rendered = backend_block(&backend, Style::plain());
        assert!(rendered.contains("skipping after 5 consecutive failures"));
        assert!(rendered.contains("1m"), "got: {rendered}");
    }

    #[test]
    fn the_balance_counts_pools_and_says_which_are_free() {
        let rendered = balance_block(
            &BalanceView {
                available: 3,
                free_available: 2,
                unknown: 1,
                unavailable: 0,
                next_available_at: None,
                spend_today_usd: Some(1.234),
                cache_hit_rate: None,
                cache_exchanges: 0,
                spend_cap: None,
                subscription_used: Vec::new(),
            },
            Style::plain(),
        );
        assert!(rendered.contains("3 pool(s) available (2 already paid for)"));
        assert!(rendered.contains("1 not yet reporting"));
        assert!(rendered.contains("$1.23"));
    }

    #[test]
    fn a_pool_that_has_not_reported_is_never_counted_as_available() {
        // The whole reason `unknown` is a separate field. Folding it into
        // `available` would be the same fabrication the headroom column exists
        // to avoid.
        let rendered = balance_block(
            &BalanceView {
                available: 0,
                free_available: 0,
                unknown: 2,
                ..BalanceView::default()
            },
            Style::plain(),
        );
        assert!(rendered.contains("2 not yet reporting"));
        assert!(!rendered.contains("available"), "got: {rendered}");
    }

    #[test]
    fn an_unmeasured_spend_shows_no_figure_at_all() {
        // `None` means the ledger is off — which is not the same as zero, and
        // printing "$0.00" would tell the user something we do not know.
        let rendered = balance_block(
            &BalanceView {
                available: 1,
                free_available: 1,
                spend_today_usd: None,
                ..BalanceView::default()
            },
            Style::plain(),
        );
        assert!(!rendered.contains('$'), "got: {rendered}");
    }

    #[test]
    fn colour_never_changes_which_words_are_printed() {
        // Colour has to be findability, never information: piped to a file or
        // read by someone who cannot distinguish red from green, the screen
        // has to say the same things.
        let strip = |text: &str| {
            let mut out = String::new();
            let mut chars = text.chars();
            while let Some(c) = chars.next() {
                if c == '\x1b' {
                    for c in chars.by_ref() {
                        if c == 'm' {
                            break;
                        }
                    }
                } else {
                    out.push(c);
                }
            }
            out
        };
        let mut backend = view(HeadroomView::Observed {
            used_pct: 95.0,
            observed_secs_ago: 10,
            resets_in_secs: Some(600),
        });
        backend.health = HealthView {
            circuit: "open".into(),
            consecutive_failures: 3,
            retry_in_secs: Some(30),
        };
        let mut view = status_view(vec![backend]);
        view.usage = usage_fixture();
        view.update = UpdateStatus::Available {
            latest: "9.9.9".into(),
            summary: None,
            upgrade_command: Some("brew upgrade ironwire".into()),
        };

        let coloured = status(&view, Style::resolve(crate::style::ColorChoice::Always));
        assert!(
            coloured.contains('\x1b'),
            "the fixture must exercise colour"
        );
        assert_eq!(strip(&coloured), status(&view, Style::plain()));
    }
}

#[cfg(test)]
fn usage_fixture() -> UsageReport {
    use ironwire_usage::{BurnRate, Ceiling, Projection, TokenCounts};

    UsageReport {
        sessions: vec![SessionUsage {
            backend: "claude-sub".into(),
            started_at: chrono::Utc::now() - chrono::Duration::hours(1),
            closes_at: chrono::Utc::now() + chrono::Duration::hours(4),
            elapsed_minutes: 60.0,
            remaining_minutes: 240.0,
            exchanges: 12,
            without_usage: 1,
            tokens: TokenCounts {
                input: 1_000,
                output: 4_000,
                cache_read: 95_000,
                cache_write: 0,
            },
            cost_usd: 2.0,
            models: vec!["claude-opus-4-6".into()],
            burn: Some(BurnRate {
                tokens_per_minute: 10_000.0,
                cost_per_hour: 12.0,
            }),
            hourly_tokens_per_minute: Some(1_666.0),
            projection: Some(Projection {
                total_tokens: 2_500_000,
                total_cost_usd: 50.0,
                remaining_minutes: 240.0,
            }),
            ceiling: Some(Ceiling {
                tokens: 200_000,
                basis: Basis::SelfCalibrated,
                description: "your own p90 over 14 past session(s)".into(),
                unverified: false,
            }),
            used_pct: Some(50.0),
            exhausts_in_minutes: Some(10.0),
        }],
        completed_sessions: 14,
        p90: None,
        history_hours: 192,
        session_hours: 5,
    }
}

#[cfg(test)]
mod usage_tests {
    use super::*;
    use ironwire_usage::{BurnRate, Ceiling, SessionUsage, TokenCounts};

    fn session() -> SessionUsage {
        usage_fixture().sessions.remove(0)
    }

    fn render(session: SessionUsage) -> String {
        usage_block(
            &UsageReport {
                sessions: vec![session],
                ..usage_fixture()
            },
            Style::plain(),
        )
    }

    #[test]
    fn the_section_says_the_numbers_are_ours_and_not_the_providers() {
        // The line above it is a rate-limit header. This one is arithmetic on
        // our own traffic, and a reader who conflates the two ends up trusting
        // an extrapolation the way they would trust the provider.
        let rendered = render(session());
        assert!(rendered.contains("measured from IronWire's own ledger"));
        assert!(rendered.contains("not reported by the provider"));
    }

    #[test]
    fn a_window_on_course_to_run_out_early_says_when() {
        let rendered = render(session());
        assert!(rendered.contains("you reach your usual ceiling in 10m"));
        assert!(
            rendered.contains("3h50m before the window closes"),
            "{rendered}"
        );
    }

    #[test]
    fn a_window_that_will_outlast_the_burn_says_the_reassuring_thing() {
        let mut session = session();
        session.exhausts_in_minutes = Some(600.0);
        let rendered = render(session);
        assert!(rendered.contains("the window closes before you reach"));
        assert!(!rendered.contains("before the window closes"), "{rendered}");
    }

    #[test]
    fn with_no_ceiling_there_is_no_bar_and_no_percentage() {
        // Drawing an empty bar against nothing implies a limit we do not have.
        let mut session = session();
        session.ceiling = None;
        session.used_pct = None;
        session.exhausts_in_minutes = None;
        let rendered = render(session);
        assert!(!rendered.contains('%'), "{rendered}");
        assert!(!rendered.contains('█'), "{rendered}");
        assert!(rendered.contains("nothing to compare against yet"));
        assert!(rendered.contains("usage.plan"));
    }

    #[test]
    fn a_declared_limit_is_named_as_the_users_own_claim() {
        let mut session = session();
        session.ceiling = Some(Ceiling {
            tokens: 88_000,
            basis: Basis::Declared,
            description: "the Max 5× limit you declared".into(),
            unverified: false,
        });
        let rendered = render(session);
        assert!(rendered.contains("the Max 5× limit you declared"));
        assert!(rendered.contains("you reach your declared limit"));
    }

    #[test]
    fn a_ceiling_its_own_source_calls_a_guess_is_flagged() {
        let mut session = session();
        session.ceiling = Some(Ceiling {
            tokens: 19_000,
            basis: Basis::Declared,
            description: "the Team limit you declared".into(),
            unverified: true,
        });
        assert!(render(session).contains("unverified"));
    }

    #[test]
    fn a_window_with_too_little_traffic_says_so_rather_than_showing_zero() {
        // "0 tokens/min" reads as "you have stopped", which is a different
        // claim from "one request is not an interval".
        let mut session = session();
        session.burn = None;
        session.projection = None;
        session.exhausts_in_minutes = None;
        let rendered = render(session);
        assert!(rendered.contains("not enough traffic in this window"));
        assert!(!rendered.contains("0.0 tokens/min"), "{rendered}");
    }

    #[test]
    fn exchanges_the_provider_said_nothing_about_are_declared_as_missing() {
        // Otherwise the token total silently undercounts and reads as complete.
        assert!(render(session()).contains("1 with no usage reported"));
    }

    #[test]
    fn an_empty_report_renders_nothing_at_all() {
        // With capture off there is no section, not an empty one.
        assert_eq!(usage_block(&UsageReport::default(), Style::plain()), "");
    }

    #[test]
    fn the_rate_and_the_projection_are_both_shown() {
        let rendered = render(session());
        assert!(rendered.contains("10.0k tokens/min"));
        assert!(rendered.contains("$12.00/hour"));
        assert!(rendered.contains("1.7k/min last hour"));
        assert!(rendered.contains("2.5M tokens by the time it closes"));
    }

    #[test]
    fn the_used_line_counts_every_kind_of_token() {
        // Cache reads are most of what a coding agent sends; leaving them out
        // would understate a long session by an order of magnitude.
        let mut session = session();
        session.tokens = TokenCounts {
            input: 1_000,
            output: 4_000,
            cache_read: 95_000,
            cache_write: 100_000,
        };
        session.burn = Some(BurnRate {
            tokens_per_minute: 1.0,
            cost_per_hour: 0.0,
        });
        assert!(render(session).contains("200.0k tokens"));
    }
}

#[cfg(test)]
mod preview {
    use super::*;

    /// Not an assertion — a way to look at the screen. `cargo test -p ironwire
    /// preview -- --nocapture --ignored`.
    #[test]
    #[ignore = "prints the status screen for a human to look at"]
    fn the_status_screen() {
        let mut view = tests::status_view(vec![BackendView {
            id: "claude-sub".into(),
            name: "Claude subscription".into(),
            kind: "subscription".into(),
            authenticated: true,
            consented: true,
            detail: None,
            headroom: HeadroomView::Observed {
                used_pct: 74.0,
                observed_secs_ago: 40,
                resets_in_secs: Some(8_040),
            },
            health: HealthView::default(),
            models: vec!["claude-opus-4-6".into(), "claude-sonnet-4-6".into()],
            connect_command: None,
        }]);
        view.balance = BalanceView {
            available: 2,
            free_available: 1,
            unknown: 1,
            unavailable: 0,
            next_available_at: None,
            spend_today_usd: Some(0.0),
            cache_hit_rate: Some(0.97),
            cache_exchanges: 412,
            spend_cap: None,
            subscription_used: vec![ironwire_proxy::control::SubscriptionUse {
                backend: "claude-sub".into(),
                name: "Claude subscription".into(),
                used_pct: Some(74.0),
                exchanges: 128,
            }],
        };
        view.usage = usage_fixture();
        println!(
            "{}",
            status(&view, Style::resolve(crate::style::ColorChoice::Always))
        );
    }
}
