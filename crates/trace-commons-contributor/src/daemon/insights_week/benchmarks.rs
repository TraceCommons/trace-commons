//! The `insights_benchmarks` answer over the counter store: a child of
//! `insights_week` so it reads the store as `model_answers_value` does,
//! without widening the store's types.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use super::{
    CounterPass, CounterRow, FEED_COUNTER_PASS, KeyedDigest, NeverFolders, never_digests,
    route_map, unreadable_value,
};
use crate::insights::benchmarks::{BenchTool, benchmarks};
use crate::insights::session_labels::SessionLabel;
use crate::insights::week_rollup::SessionInput;

impl CounterPass {
    /// The `insights_benchmarks` answer: per category, each model's kept
    /// share, list-priced cost and active time over the stored sessions in
    /// the `weeks` ending `now`, for `tool` or both. Gated, joined to the
    /// person's labels and to the route tallies as `model_answers_value`
    /// is; no NEAR AI charge is read.
    pub(crate) fn benchmarks_value(
        &self,
        enabled: bool,
        never_project_keys: &NeverFolders,
        routing_available: bool,
        weeks: u32,
        tool: Option<BenchTool>,
        now: DateTime<Utc>,
    ) -> serde_json::Value {
        if !enabled {
            return serde_json::json!({"enabled": false, "feed": FEED_COUNTER_PASS});
        }
        let (key, store) = match self.visible_store() {
            Ok(found) => found,
            Err(reason) => return unreadable_value(reason),
        };
        let (inputs, routes, labels) = match (&key, &store) {
            (Some(key), Some(store)) => {
                let never = never_digests(key, never_project_keys);
                let mut keyed: Vec<(&KeyedDigest, &CounterRow)> = store
                    .rows
                    .iter()
                    .filter(|(_, row)| !never.contains(&row.project))
                    .collect();
                keyed.sort_by_key(|(_, row)| row.seq);
                let user_labels = self.readable_labels(&store.key_fingerprint);
                let labels: BTreeMap<String, SessionLabel> = keyed
                    .iter()
                    .filter_map(|(session, row)| {
                        user_labels
                            .get(*session)
                            .map(|label| (row.reference(), *label))
                    })
                    .collect();
                let rows: Vec<&CounterRow> = keyed.into_iter().map(|(_, row)| row).collect();
                let routes = route_map(&rows, &store.routing, routing_available);
                let inputs: Vec<SessionInput> =
                    rows.iter().map(|row| row.session_input()).collect();
                (inputs, routes, labels)
            }
            _ => (Vec::new(), BTreeMap::new(), BTreeMap::new()),
        };
        let answer = benchmarks(
            &inputs,
            &routes,
            &labels,
            crate::insights::pricing_catalog::shared_pricing_catalog(),
            weeks,
            tool,
            now,
        );
        let tables: Vec<serde_json::Value> = answer
            .pricing_tables
            .iter()
            .map(|table| serde_json::json!({"table_id": table.table_id, "version": table.version}))
            .collect();
        serde_json::json!({
            "enabled": true,
            "feed": FEED_COUNTER_PASS,
            "readable": true,
            "updated_at": self.updated_at(),
            "window_start": answer.window_start.to_rfc3339(),
            "window_end": answer.window_end.to_rfc3339(),
            "weeks": answer.weeks,
            "tool": tool,
            "pricing": {
                "basis": crate::insights::session_pricing::PRICING_BASIS_API_LIST,
                "tables": tables,
            },
            "sessions": answer.sessions,
            "min_sessions": answer.min_sessions,
            "categories": answer.categories,
            "left_out": answer.left_out,
        })
    }
}
