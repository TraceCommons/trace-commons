// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Classify a contribution's text leaves concurrently, without changing what
//! the pipeline produces.
//!
//! # Why
//!
//! `redact_trace` walks a contribution leaf by leaf -- every event's content,
//! every metadata string, every structured-payload string and key -- and
//! awaits the prose classifier on each in turn. Against NEAR AI each call
//! costs about 1.3 s however small its input, so a trace's wall time is its
//! leaf count times that: measured locally against a stub with the
//! endpoint's latency, 241 KB in one field took 9.7 s and the same 241 KB
//! across 60 tool results took 85.5 s, with the witness's own CPU under 1% of
//! either. Most of those calls carry under 200 bytes.
//!
//! # How, and why the output cannot change
//!
//! Two passes over the same raw contribution, through the unmodified
//! protocol pipeline:
//!
//! 1. **Record.** The pipeline runs with an adapter that records each text it
//!    is asked to classify and answers "nothing found". The classifier stage
//!    never feeds the deterministic stage -- placeholder numbering, the scan
//!    budget and every later classifier input are derived from the raw
//!    contribution alone -- so the texts recorded are exactly the texts the
//!    real pass will ask about.
//! 2. **Classify.** The distinct texts go to the real adapter, a bounded
//!    number of requests at once, under a permit pool shared by every request
//!    in the process, since every request spends the same classifier key.
//!    An adapter that batches (`PrivacyFilterAdapter::redact_texts`) is
//!    handed several texts per request.
//! 3. **Replay.** The pipeline runs again with an adapter that answers each
//!    text from those results. This is the pass whose output is returned, and
//!    it is the same call the sequential witness makes; only where each answer
//!    came from differs. A text with no answer is a refusal. The replay can
//!    only meet one if classification stopped abnormally, because the record
//!    pass saw every text the replay reaches before its first refusal.
//!
//! A classifier failure stays a failure: the first failing text in pipeline
//! order is answered with its error in the replay, which then fails exactly
//! where the sequential pass would have, and the witness refuses.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::Semaphore;
use trace_commons_protocol::trace_contribution::{
    DeterministicTraceRedactor, PrivacyFilterAdapter, PrivacyFilterBackendTag,
    RawTraceContribution, SafePrivacyFilterRedaction, TraceContributionError,
    redaction_pipeline_version,
};

use super::{ContributionRedactor, RedactedContribution, SeamUnavailable, refuse_on_panic};

/// Classifier calls in flight at once, across the whole process.
///
/// Two, from the one controlled measurement there is: on 2026-08-28, against
/// the same NEAR AI key, two concurrent requests passed 2/2 and four failed
/// 0/4 within a second, and the burst briefly poisoned serial traffic after
/// it. Today's witness already allows four overlapping calls -- one per
/// request slot -- so two is tighter than what is deployed, while still
/// halving a single large request's wall time. That measurement predates the
/// endpoint's 2026-09-03 limit change and has not been repeated; raise this
/// only after re-measuring with synthetic text, never with the production key
/// while a driver is using it.
pub const DEFAULT_MAX_CLASSIFY_IN_FLIGHT: usize = 2;

type Answer = Result<Option<SafePrivacyFilterRedaction>, TraceContributionError>;

fn answer_missing() -> TraceContributionError {
    TraceContributionError::RedactionFailed {
        reason: "witness_classify_answer_missing".to_string(),
    }
}

fn permits_closed() -> TraceContributionError {
    TraceContributionError::RedactionFailed {
        reason: "witness_classify_permits_closed".to_string(),
    }
}

/// Poll `future` once: `Some` if it is ready, `None` if it is pending (and
/// so, for a fair lock, now registered in its wait queue).
async fn futures_poll_once<F: std::future::Future + Unpin>(future: F) -> Option<F::Output> {
    let mut future = future;
    std::future::poll_fn(|cx| {
        std::task::Poll::Ready(match std::pin::Pin::new(&mut future).poll(cx) {
            std::task::Poll::Ready(value) => Some(value),
            std::task::Poll::Pending => None,
        })
    })
    .await
}

/// Records every text the pipeline asks about, in order, and finds nothing.
#[derive(Default)]
struct RecordingAdapter {
    seen: Mutex<Vec<String>>,
}

impl RecordingAdapter {
    /// The recorded texts, first occurrence order, duplicates removed.
    fn distinct(&self) -> Vec<String> {
        let seen = std::mem::take(&mut *self.seen.lock().unwrap_or_else(|p| p.into_inner()));
        let mut kept = std::collections::HashSet::new();
        seen.into_iter()
            .filter(|text| kept.insert(text.clone()))
            .collect()
    }
}

#[async_trait]
impl PrivacyFilterAdapter for RecordingAdapter {
    async fn redact_text(&self, text: &str) -> Answer {
        self.seen
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(text.to_string());
        Ok(None)
    }
}

/// Answers from the classify pass, falling back to the real adapter (under
/// the shared permits) for any text that pass did not reach.
struct AnsweringAdapter {
    answers: HashMap<String, Answer>,
}

#[async_trait]
impl PrivacyFilterAdapter for AnsweringAdapter {
    async fn redact_text(&self, text: &str) -> Answer {
        // A text with no answer is a refusal, never a fresh classifier call
        // and never a pass. The record pass saw every text the replay can
        // reach before its first refusal, so this is reached only when
        // classification stopped abnormally.
        self.answers
            .get(text)
            .cloned()
            .unwrap_or_else(|| Err(answer_missing()))
    }
}

/// The full pipeline over a contribution, with its classifier calls made
/// concurrently. Same output as
/// [`super::PipelineContributionRedaction::with_privacy_filter`] for the same
/// classifier answers; see the module docs.
pub struct PrefetchingContributionRedaction {
    known_path_prefixes: Vec<String>,
    adapter: Arc<dyn PrivacyFilterAdapter>,
    backend: PrivacyFilterBackendTag,
    policy_version: String,
    permits: Arc<Semaphore>,
    per_request: usize,
    /// How long a request's classify pass may take; see
    /// [`Self::with_classify_budget`].
    budget: Option<std::time::Duration>,
    /// Whose turn it is to classify; see [`ClassifyQueue`].
    queue: Arc<ClassifyQueue>,
}

/// Requests take turns at the classifier, first come first served.
///
/// One request classifies at a time, using every classify permit it can;
/// the next waits its turn. That keeps admitted requests in arrival order,
/// so the first request to arrive is the first to finish, and it makes the
/// wait ahead of a new request predictable: roughly the requests ahead of it
/// times the recent time a request spends classifying. The witness's
/// admission check ([`super::http::WitnessLoadBound::with_classify_queue`])
/// uses that estimate to answer busy at once rather than admit a request
/// that would only time out.
///
/// `tokio::sync::Mutex` is fair: waiters are woken in the order they asked.
pub struct ClassifyQueue {
    turn: tokio::sync::Mutex<()>,
    waiting: std::sync::atomic::AtomicUsize,
    /// Requests that have joined the queue since start.
    joined: std::sync::atomic::AtomicU64,
    /// Exponentially weighted mean of a request's turn, in milliseconds; 0
    /// until a request has finished one.
    mean_turn_ms: std::sync::atomic::AtomicU64,
}

impl Default for ClassifyQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl ClassifyQueue {
    pub fn new() -> Self {
        Self {
            turn: tokio::sync::Mutex::new(()),
            waiting: std::sync::atomic::AtomicUsize::new(0),
            joined: std::sync::atomic::AtomicU64::new(0),
            mean_turn_ms: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Requests waiting for their turn right now.
    pub fn waiting(&self) -> usize {
        self.waiting.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Requests that have joined the queue since the process started.
    pub fn joined(&self) -> u64 {
        self.joined.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The recent time a request spends classifying, once one has.
    pub fn mean_turn(&self) -> Option<std::time::Duration> {
        match self.mean_turn_ms.load(std::sync::atomic::Ordering::Relaxed) {
            0 => None,
            ms => Some(std::time::Duration::from_millis(ms)),
        }
    }

    /// Whether a request arriving behind `ahead` others would, at the recent
    /// pace, still be waiting when `budget` runs out. Unknown pace: no.
    pub fn would_exceed(&self, ahead: usize, budget: std::time::Duration) -> bool {
        self.mean_turn()
            .is_some_and(|turn| turn.saturating_mul(ahead as u32 + 1) > budget)
    }

    /// Seed the pace, for tests.
    pub fn record_turn(&self, took: std::time::Duration) {
        let took_ms = u64::try_from(took.as_millis()).unwrap_or(u64::MAX).max(1);
        // Weight 1/4 on the newest turn: recent enough to follow the
        // endpoint slowing down, smooth enough that one fast request does
        // not open the gate for a queue of slow ones.
        let mut mean = self.mean_turn_ms.load(std::sync::atomic::Ordering::Relaxed);
        loop {
            let next = if mean == 0 {
                took_ms
            } else {
                (mean.saturating_mul(3).saturating_add(took_ms)) / 4
            };
            match self.mean_turn_ms.compare_exchange_weak(
                mean,
                next,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(current) => mean = current,
            }
        }
    }

    async fn take_turn(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.waiting
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Join the lock's wait list before counting the arrival, so a
        // request counted as joined is already in line behind every earlier
        // one.
        let lock = self.turn.lock();
        tokio::pin!(lock);
        let guard = match futures_poll_once(lock.as_mut()).await {
            Some(guard) => guard,
            None => {
                self.joined
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let guard = lock.await;
                self.waiting
                    .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                return guard;
            }
        };
        self.joined
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.waiting
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        guard
    }
}

impl PrefetchingContributionRedaction {
    /// `max_in_flight` bounds classifier calls across every request sharing
    /// this redactor, which is every request the process serves; one request
    /// may use all of them. Zero is treated as one.
    pub fn new(
        known_path_prefixes: Vec<String>,
        adapter: Arc<dyn PrivacyFilterAdapter>,
        backend: PrivacyFilterBackendTag,
        max_in_flight: usize,
    ) -> Self {
        let max_in_flight = max_in_flight.max(1);
        Self {
            known_path_prefixes,
            adapter,
            backend,
            policy_version: redaction_pipeline_version(backend),
            permits: Arc::new(Semaphore::new(max_in_flight)),
            per_request: max_in_flight,
            budget: None,
            queue: Arc::new(ClassifyQueue::new()),
        }
    }

    /// The queue requests take turns in, to share with the admission check.
    pub fn classify_queue(&self) -> Arc<ClassifyQueue> {
        self.queue.clone()
    }

    /// Give each request's classify pass a deadline this long after it
    /// starts, waiting for permits included. Classifier calls are made with
    /// [`PrivacyFilterAdapter::redact_text_before`], so a retry that could not
    /// finish by then is not started and the request refuses instead of
    /// running into the witness's request timeout.
    pub fn with_classify_budget(mut self, budget: std::time::Duration) -> Self {
        self.budget = Some(budget);
        self
    }

    fn pipeline(&self, adapter: Arc<dyn PrivacyFilterAdapter>) -> DeterministicTraceRedactor {
        DeterministicTraceRedactor::deterministic_only(self.known_path_prefixes.clone())
            .with_privacy_filter(adapter, self.backend)
    }

    /// Steps 1 and 2: record the texts, then classify them concurrently.
    async fn answers_for(&self, raw: &RawTraceContribution) -> AnsweringAdapter {
        let recorder = Arc::new(RecordingAdapter::default());
        // The record pass's own result is discarded. If it refuses (a
        // credential in metadata, a key collision), the replay refuses at the
        // same leaf; the texts recorded before that point are still worth
        // classifying ahead of it.
        let _ = self
            .pipeline(recorder.clone())
            .redact_trace_with_edits(raw.clone())
            .await;
        let texts = recorder.distinct();
        let deadline = self.budget.map(|budget| std::time::Instant::now() + budget);
        // Waiting for the turn counts against the deadline, as it should:
        // the caller's clock is running.
        let _turn = self.queue.take_turn().await;
        let turn_started = std::time::Instant::now();

        // Texts go to the adapter in chunks of what it prefers to be handed
        // at once: 1 unless it batches (NEAR AI with
        // `TRACE_NEAR_AI_PRIVACY_MAX_BATCH_INPUTS` above 1). One chunk holds
        // one permit, so the bound counts requests, not texts.
        //
        // Bounded and ordered: at most `per_request` chunks are started ahead
        // of the earliest unanswered one, and answers are taken in pipeline
        // order, so the first error seen is the first in that order,
        // everything before it has its answer, and nothing more is started
        // after it. Dropping the set (an error, or the request's timeout)
        // aborts whatever is still in flight.
        let chunk = self.adapter.preferred_batch_inputs().max(1);
        let mut chunks: Vec<Vec<String>> = Vec::new();
        for text in texts {
            match chunks.last_mut() {
                Some(last) if last.len() < chunk => last.push(text),
                _ => chunks.push(vec![text]),
            }
        }
        let mut answers = HashMap::new();
        let mut pending = tokio::task::JoinSet::new();
        let mut done: HashMap<usize, Vec<(String, Answer)>> = HashMap::new();
        let mut queue = chunks.into_iter().enumerate();
        let mut next_to_take = 0usize;
        let mut in_flight = 0usize;
        'classify: loop {
            while in_flight < self.per_request {
                let Some((index, texts)) = queue.next() else {
                    break;
                };
                let adapter = self.adapter.clone();
                let permits = self.permits.clone();
                pending.spawn(async move {
                    let results = match permits.acquire_owned().await {
                        Ok(_permit) => {
                            let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();
                            match deadline {
                                Some(deadline) => {
                                    adapter.redact_texts_before(&borrowed, deadline).await
                                }
                                None => adapter.redact_texts(&borrowed).await,
                            }
                        }
                        Err(_) => texts.iter().map(|_| Err(permits_closed())).collect(),
                    };
                    (index, texts.into_iter().zip(results).collect::<Vec<_>>())
                });
                in_flight += 1;
            }
            let Some(joined) = pending.join_next().await else {
                break;
            };
            in_flight -= 1;
            match joined {
                Ok((index, results)) => {
                    done.insert(index, results);
                }
                // A panicking classifier call: stop. The replay meets the
                // texts it left unanswered and refuses.
                Err(_) => break,
            }
            while let Some(results) = done.remove(&next_to_take) {
                next_to_take += 1;
                for (text, answer) in results {
                    let failed = answer.is_err();
                    answers.insert(text, answer);
                    if failed {
                        break 'classify;
                    }
                }
            }
        }
        self.queue.record_turn(turn_started.elapsed());
        AnsweringAdapter { answers }
    }
}

#[async_trait]
impl ContributionRedactor for PrefetchingContributionRedaction {
    async fn redact_with_edits(
        &self,
        raw: RawTraceContribution,
    ) -> Result<
        (
            RedactedContribution,
            std::collections::BTreeMap<
                uuid::Uuid,
                trace_commons_protocol::private_edit_map::PrivateRedactionEdits,
            >,
        ),
        SeamUnavailable,
    > {
        let answers = refuse_on_panic(async { Ok::<_, ()>(self.answers_for(&raw).await) }).await?;
        let (envelope, maps) = refuse_on_panic(
            self.pipeline(Arc::new(answers))
                .redact_trace_with_edits(raw),
        )
        .await?;
        Ok((
            RedactedContribution {
                envelope,
                policy_version: self.policy_version.clone(),
            },
            maps,
        ))
    }

    async fn redact(
        &self,
        raw: RawTraceContribution,
    ) -> Result<RedactedContribution, SeamUnavailable> {
        use trace_commons_protocol::trace_contribution::TraceRedactor;
        let answers = refuse_on_panic(async { Ok::<_, ()>(self.answers_for(&raw).await) }).await?;
        let envelope = refuse_on_panic(self.pipeline(Arc::new(answers)).redact_trace(raw)).await?;
        Ok(RedactedContribution {
            envelope,
            policy_version: self.policy_version.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use serde_json::json;
    use trace_commons_protocol::privacy_filter_near_ai::NearAiPrivacyFilterAdapter;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    use crate::witness_service::PipelineContributionRedaction;
    use trace_commons_protocol::trace_contribution::TraceRedactor;

    /// What the stand-in classifier reports, wherever it appears.
    const NAMES: &[&str] = &["Alice Brannigan", "Zo\u{eb} Ng", "bob@example.com"];

    /// A contribution with what the pipeline treats differently: message
    /// text, tool output with names and a credential, multibyte text,
    /// structured payloads whose keys and values are both classified,
    /// repeated text (deduplicated by the record pass), and a hostname the
    /// deterministic pass numbers, so placeholder counters are on the path.
    fn rich_contribution(events: usize) -> RawTraceContribution {
        let mut body_events = vec![json!({
            "content": "please ask Alice Brannigan about the build on build.corp.internal",
            "cost_usd": null, "latency_ms": null, "token_counts": null, "tool_name": null,
            "event_id": uuid::Uuid::from_u128(1), "event_type": "user_message",
            "structured_payload": {"state": "Completed"},
            "timestamp": "2026-11-05T00:00:00Z",
        })];
        for i in 0..events {
            let call = uuid::Uuid::from_u128(1000 + i as u128);
            body_events.push(json!({
                "cost_usd": null, "latency_ms": null, "token_counts": null, "content": null,
                "event_id": call, "event_type": "tool_call",
                "structured_payload": {"arguments": {
                    "cmd": format!("ssh build{}.corp.internal 'cat /home/zo\u{eb}/notes-{i}.txt'", i % 3),
                    "Zo\u{eb} Ng": "owner",
                    "nested": [{"who": "bob@example.com"}, "plain", i],
                }},
                "success": true, "timestamp": "2026-11-05T00:00:01Z",
                "tool_call_id": format!("call_{i}"), "tool_name": "shell",
            }));
            let output = if i % 4 == 0 {
                // Repeated verbatim across events.
                "the same tool output every fourth time, mentioning Alice Brannigan\n".repeat(3)
            } else {
                format!(
                    "line {i}: caf\u{e9} \u{4e2d}\u{6587} \u{1f600} from Zo\u{eb} Ng <bob@example.com>\n\
                     key {} at build{}.corp.internal\n{}",
                    concat!("AKIA", "QQWERTYUIOPASDFG"),
                    i % 5,
                    "filler \"quoted\" C:\\dir\\file\n".repeat(i % 7)
                )
            };
            body_events.push(json!({
                "cost_usd": null, "latency_ms": null, "token_counts": null,
                "content": output,
                "event_id": uuid::Uuid::from_u128(2000 + i as u128), "event_type": "tool_result",
                "parent_event_id": call, "structured_payload": null, "success": true,
                "timestamp": "2026-11-05T00:00:02Z",
                "tool_call_id": format!("call_{i}"), "tool_name": "shell",
            }));
        }
        serde_json::from_value(json!({
            "consent": {
                "correction_included": false, "message_text_included": true,
                "policy_version": "2026-04-24", "revocable": true,
                "routing_metadata_included": false, "scopes": ["debugging_evaluation"],
                "tool_payloads_included": true,
            },
            "contributor": {"revocation_handle": uuid::Uuid::from_u128(7)},
            "created_at": "2026-11-05T00:00:00Z",
            "embedding_analysis": null,
            "events": body_events,
            "ironclaw": {"channel": "cli", "version": "0.1.0"},
            "outcome": {"task_success": "success", "user_feedback": "none"},
            "replay": {"replay_notes": ["Synthetic, about Alice Brannigan."], "replayable": true, "required_tools": ["shell"]},
            "submission_id": uuid::Uuid::from_u128(8),
            "trace_id": uuid::Uuid::from_u128(9),
            "value": {"credit_points_pending": 0.0, "submission_score": 0.0},
        }))
        .expect("the fixture is a raw contribution")
    }

    /// A NEAR AI stand-in that spans every name at the request's own
    /// codepoint offsets, after a delay that varies with the request, so
    /// concurrent answers come back out of order.
    fn name_spans(request: &Request) -> ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        let inputs: Vec<&str> = match &body["input"] {
            serde_json::Value::String(one) => vec![one.as_str()],
            serde_json::Value::Array(many) => many.iter().map(|v| v.as_str().unwrap()).collect(),
            other => panic!("unexpected input {other}"),
        };
        let data: Vec<serde_json::Value> = inputs
            .iter()
            .map(|input| {
                let mut spans = Vec::new();
                for name in NAMES {
                    for (byte, found) in input.match_indices(name) {
                        let start = input[..byte].chars().count();
                        spans.push(json!({
                            "category": "private_person", "start": start,
                            "end": start + found.chars().count(), "score": 0.9, "text": found,
                        }));
                    }
                }
                spans.sort_by_key(|span| span["start"].as_u64());
                json!({"spans": spans})
            })
            .collect();
        ResponseTemplate::new(200)
            .set_delay(Duration::from_millis((request.body.len() % 7) as u64 * 3))
            .set_body_json(json!({"data": data}))
    }

    async fn classifier() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/privacy/classify"))
            .respond_with(name_spans)
            .mount(&server)
            .await;
        server
    }

    /// A fresh adapter, so neither pass benefits from the other's cache.
    fn near_ai(server: &MockServer) -> Arc<dyn PrivacyFilterAdapter> {
        near_ai_batching(server, 1)
    }

    fn near_ai_batching(server: &MockServer, batch: usize) -> Arc<dyn PrivacyFilterAdapter> {
        Arc::new(
            NearAiPrivacyFilterAdapter::new(
                server.uri(),
                "openai/privacy-filter",
                "stub-not-a-key",
                Duration::from_secs(10),
                16 * 1024 * 1024,
            )
            .unwrap()
            .with_max_batch_inputs(batch),
        )
    }

    /// The property this module exists under: the same envelope, byte for
    /// byte, and the same private edit maps, as the sequential pipeline, for
    /// the same classifier answers.
    #[tokio::test]
    async fn the_envelope_and_edit_maps_are_identical_to_the_sequential_pass() {
        let server = classifier().await;
        for events in [0, 1, 7, 40] {
            let raw = rich_contribution(events);
            let sequential = PipelineContributionRedaction::with_privacy_filter(
                Vec::new(),
                near_ai(&server),
                PrivacyFilterBackendTag::NearAi,
            );
            let prefetching = PrefetchingContributionRedaction::new(
                Vec::new(),
                near_ai(&server),
                PrivacyFilterBackendTag::NearAi,
                4,
            );

            let (expected, expected_maps) = sequential
                .redact_with_edits(raw.clone())
                .await
                .expect("redacts");
            let (actual, actual_maps) = prefetching
                .redact_with_edits(raw.clone())
                .await
                .expect("redacts");
            let expected_bytes = serde_json::to_vec(&expected.envelope).unwrap();
            assert_eq!(
                serde_json::to_vec(&actual.envelope).unwrap(),
                expected_bytes
            );
            assert!(
                actual_maps == expected_maps,
                "edit maps differ at {events} events"
            );
            assert!(
                !expected_maps.is_empty(),
                "the maps must carry edits to compare"
            );
            assert_eq!(actual.policy_version, expected.policy_version);

            let plain = prefetching.redact(raw.clone()).await.expect("redacts");
            assert_eq!(serde_json::to_vec(&plain.envelope).unwrap(), expected_bytes);

            // And with small texts batched into input arrays.
            let batching = PrefetchingContributionRedaction::new(
                Vec::new(),
                near_ai_batching(&server, 8),
                PrivacyFilterBackendTag::NearAi,
                4,
            );
            let (batched, batched_maps) = batching
                .redact_with_edits(raw.clone())
                .await
                .expect("redacts");
            assert_eq!(
                serde_json::to_vec(&batched.envelope).unwrap(),
                expected_bytes
            );
            assert!(
                batched_maps == expected_maps,
                "batched edit maps differ at {events} events"
            );

            // Controls: the classifier and the deterministic pass both ran.
            let text = String::from_utf8(expected_bytes).unwrap();
            for name in NAMES {
                assert!(!text.contains(name), "{name} survived at {events} events");
            }
            assert!(!text.contains(concat!("AKIA", "QQWERTYUIOPASDFG")));
        }
    }

    /// A stand-in classifier that measures overlap.
    struct Overlap {
        now: AtomicUsize,
        peak: AtomicUsize,
        calls: AtomicUsize,
        fail_on: Option<&'static str>,
    }

    impl Overlap {
        fn new(fail_on: Option<&'static str>) -> Arc<Self> {
            Arc::new(Self {
                now: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                calls: AtomicUsize::new(0),
                fail_on,
            })
        }
    }

    #[async_trait]
    impl PrivacyFilterAdapter for Overlap {
        async fn redact_text(&self, text: &str) -> Answer {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let now = self.now.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.now.fetch_sub(1, Ordering::SeqCst);
            if self.fail_on.is_some_and(|marker| text.contains(marker)) {
                return Err(TraceContributionError::TransientRedactionFailed {
                    reason: "classifier down".to_string(),
                });
            }
            Ok(None)
        }
    }

    /// Calls overlap up to the bound and never past it, including across
    /// requests sharing one redactor -- the bound is per process, because
    /// every request spends the same classifier key.
    #[tokio::test]
    async fn classifier_calls_overlap_up_to_the_process_wide_bound() {
        let overlap = Overlap::new(None);
        let redactor = Arc::new(PrefetchingContributionRedaction::new(
            Vec::new(),
            overlap.clone(),
            PrivacyFilterBackendTag::NearAi,
            3,
        ));
        let first = redactor.redact(rich_contribution(20));
        let second = redactor.redact(rich_contribution(21));
        let (first, second) = tokio::join!(first, second);
        first.expect("redacts");
        second.expect("redacts");
        // The bound is exact; reaching it is timing-dependent, so only
        // require that calls did overlap.
        let peak = overlap.peak.load(Ordering::SeqCst);
        assert!((2..=3).contains(&peak), "peak overlap {peak}");

        let single = Overlap::new(None);
        PrefetchingContributionRedaction::new(
            Vec::new(),
            single.clone(),
            PrivacyFilterBackendTag::NearAi,
            1,
        )
        .redact(rich_contribution(10))
        .await
        .expect("redacts");
        assert_eq!(single.peak.load(Ordering::SeqCst), 1);
    }

    /// Each distinct text is classified once per request: the record pass
    /// deduplicates, and the replay is answered without a second call.
    #[tokio::test]
    async fn each_distinct_text_is_classified_once() {
        let overlap = Overlap::new(None);
        let raw = rich_contribution(12);
        let recorder = Arc::new(RecordingAdapter::default());
        DeterministicTraceRedactor::deterministic_only(Vec::new())
            .with_privacy_filter(recorder.clone(), PrivacyFilterBackendTag::NearAi)
            .redact_trace_with_edits(raw.clone())
            .await
            .expect("redacts");
        let seen = recorder.seen.lock().unwrap().len();
        let distinct = recorder.distinct().len();
        assert!(distinct < seen, "the fixture must repeat some text");

        PrefetchingContributionRedaction::new(
            Vec::new(),
            overlap.clone(),
            PrivacyFilterBackendTag::NearAi,
            4,
        )
        .redact(raw)
        .await
        .expect("redacts");
        assert_eq!(overlap.calls.load(Ordering::SeqCst), distinct);
    }

    /// Fail-closed: a classifier failure on any text refuses the whole
    /// contribution, and calls stop being started once it is seen.
    #[tokio::test]
    async fn a_classifier_failure_refuses_the_contribution() {
        let overlap = Overlap::new(Some("line 3:"));
        let redactor = PrefetchingContributionRedaction::new(
            Vec::new(),
            overlap.clone(),
            PrivacyFilterBackendTag::NearAi,
            2,
        );
        assert!(redactor.redact(rich_contribution(30)).await.is_err());
        let calls = overlap.calls.load(Ordering::SeqCst);
        assert!(
            redactor
                .redact_with_edits(rich_contribution(30))
                .await
                .is_err()
        );
        let recorder = Arc::new(RecordingAdapter::default());
        let _ = DeterministicTraceRedactor::deterministic_only(Vec::new())
            .with_privacy_filter(recorder.clone(), PrivacyFilterBackendTag::NearAi)
            .redact_trace(rich_contribution(30))
            .await;
        let distinct = recorder.distinct().len();
        assert!(
            calls < distinct,
            "{calls} of {distinct} texts classified: classification continued past the failure"
        );
    }

    /// A text the classify pass left unanswered is a refusal in the replay,
    /// not a fresh call and not a pass.
    #[tokio::test]
    async fn an_unanswered_text_is_a_refusal() {
        let answering = AnsweringAdapter {
            answers: HashMap::from([("known".to_string(), Ok(None))]),
        };
        assert!(answering.redact_text("known").await.is_ok());
        let error = answering.redact_text("never classified").await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("witness_classify_answer_missing")
        );
    }

    /// With a batching adapter, the permits count requests: a contribution
    /// whose small texts fit a few batches makes far fewer calls.
    #[tokio::test]
    async fn a_batching_adapter_is_handed_several_texts_per_request() {
        let server = classifier().await;
        PrefetchingContributionRedaction::new(
            Vec::new(),
            near_ai_batching(&server, 16),
            PrivacyFilterBackendTag::NearAi,
            2,
        )
        .redact(rich_contribution(30))
        .await
        .expect("redacts");
        let batched = server.received_requests().await.unwrap().len();
        server.reset().await;
        Mock::given(method("POST"))
            .and(path("/privacy/classify"))
            .respond_with(name_spans)
            .mount(&server)
            .await;
        PrefetchingContributionRedaction::new(
            Vec::new(),
            near_ai(&server),
            PrivacyFilterBackendTag::NearAi,
            2,
        )
        .redact(rich_contribution(30))
        .await
        .expect("redacts");
        let single = server.received_requests().await.unwrap().len();
        assert!(
            batched * 4 < single,
            "batched {batched} requests, single {single}"
        );
    }

    /// A stand-in classifier that logs which request each call served, and
    /// the overlap.
    struct Tagged {
        log: Mutex<Vec<String>>,
        now: AtomicUsize,
        peak: AtomicUsize,
    }

    #[async_trait]
    impl PrivacyFilterAdapter for Tagged {
        async fn redact_text(&self, text: &str) -> Answer {
            let now = self.now.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(5)).await;
            self.now.fetch_sub(1, Ordering::SeqCst);
            if let Some(tag) = text.split_whitespace().find(|word| word.starts_with("req")) {
                self.log.lock().unwrap().push(tag.to_string());
            }
            Ok(None)
        }
    }

    /// A contribution whose every tool output names `tag`.
    fn tagged_contribution(tag: &str) -> RawTraceContribution {
        let mut raw = rich_contribution(6);
        for event in &mut raw.events {
            if let Some(content) = event.content.as_mut() {
                *content = format!("{tag} {content}");
            }
        }
        raw
    }

    /// Admitted requests take turns at the classifier in arrival order: every
    /// call for the first request is made before any for the second. And
    /// however many requests wait, no more than the bound are in flight.
    #[tokio::test]
    async fn requests_classify_in_arrival_order_and_never_over_the_bound() {
        let tagged = Arc::new(Tagged {
            log: Mutex::new(Vec::new()),
            now: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        });
        let redactor = Arc::new(PrefetchingContributionRedaction::new(
            Vec::new(),
            tagged.clone(),
            PrivacyFilterBackendTag::NearAi,
            2,
        ));
        let mut handles = Vec::new();
        let queue = redactor.classify_queue();
        for i in 0..16 {
            let redactor = redactor.clone();
            handles.push(tokio::spawn(async move {
                redactor
                    .redact(tagged_contribution(&format!("req{i:02}")))
                    .await
            }));
            // Arrive in order: wait until this one has joined the queue.
            tokio::time::timeout(Duration::from_secs(10), async {
                while queue.joined() < i as u64 + 1 {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("each request joins the queue");
        }
        for handle in handles {
            handle.await.expect("no panic").expect("redacts");
        }
        let log = tagged.log.lock().unwrap().clone();
        let mut order: Vec<String> = Vec::new();
        for tag in &log {
            if order.last() != Some(tag) {
                order.push(tag.clone());
            }
        }
        let expected: Vec<String> = (0..16).map(|i| format!("req{i:02}")).collect();
        assert_eq!(order, expected, "requests interleaved or ran out of order");
        assert!(tagged.peak.load(Ordering::SeqCst) <= 2);
        assert!(redactor.classify_queue().mean_turn().is_some());
    }
}
