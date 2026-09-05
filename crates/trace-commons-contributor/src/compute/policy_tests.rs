use super::*;

fn healthy(now: Instant) -> Observation {
    Observation {
        epoch: 0,
        sequence: 1,
        observed_at: now,
        power: PowerSource::Ac,
        low_power_mode: Some(false),
        thermal: ThermalState::Nominal,
        memory: MemoryPressure::Normal,
    }
}

fn running(now: Instant) -> ResourcePolicy {
    let mut policy = ResourcePolicy::default();
    assert!(!policy.observe(healthy(now), now).may_run);
    assert!(policy.request_run(false, now).may_run);
    policy
}

#[test]
fn all_resource_combinations_fail_closed() {
    let now = Instant::now();
    for power in [
        PowerSource::Ac,
        PowerSource::Battery,
        PowerSource::Ups,
        PowerSource::Unknown,
    ] {
        for low_power_mode in [Some(false), Some(true), None] {
            for thermal in [
                ThermalState::Nominal,
                ThermalState::Fair,
                ThermalState::Serious,
                ThermalState::Critical,
                ThermalState::Unknown,
            ] {
                for memory in [
                    MemoryPressure::Normal,
                    MemoryPressure::Warning,
                    MemoryPressure::Critical,
                    MemoryPressure::Unknown,
                ] {
                    let mut policy = ResourcePolicy::default();
                    policy.observe(
                        Observation {
                            power,
                            low_power_mode,
                            thermal,
                            memory,
                            ..healthy(now)
                        },
                        now,
                    );
                    let decision = policy.request_run(false, now);
                    assert_eq!(
                        decision.may_run,
                        power == PowerSource::Ac
                            && low_power_mode == Some(false)
                            && thermal == ThermalState::Nominal
                            && memory == MemoryPressure::Normal
                    );
                    if let Some(reason) = decision.reason {
                        assert!(!reason.detail().is_empty());
                    }
                    let mut active = running(now);
                    let later = now + Duration::from_secs(1);
                    let updated = active.observe(
                        Observation {
                            sequence: 2,
                            power,
                            low_power_mode,
                            thermal,
                            memory,
                            ..healthy(later)
                        },
                        later,
                    );
                    assert_eq!(updated.may_run, decision.may_run);
                    assert_eq!(updated.stop.is_some(), !decision.may_run);
                    if thermal == ThermalState::Critical || memory == MemoryPressure::Critical {
                        assert_eq!(updated.stop.unwrap().urgency, StopUrgency::Urgent);
                    }
                }
            }
        }
    }
}

#[test]
fn expiry_is_exact_and_recovery_requires_reap_then_resume() {
    let now = Instant::now();
    let mut policy = running(now);
    assert!(
        policy
            .evaluate(now + OBSERVATION_TTL - Duration::from_nanos(1))
            .may_run
    );
    let expired = policy.evaluate(now + OBSERVATION_TTL);
    assert_eq!(expired.stop.unwrap().reason, PolicyReason::StaleObservation);
    let later = now + OBSERVATION_TTL;
    policy.observe(
        Observation {
            sequence: 2,
            ..healthy(later)
        },
        later,
    );
    assert!(!policy.request_run(false, later).may_run);
    assert!(!policy.confirm_stopped(later).may_run);
    assert!(policy.request_run(false, later).may_run);
}

#[test]
fn fresh_arrival_cannot_hide_expiry_between_ticks() {
    let now = Instant::now();
    let mut policy = running(now);
    let later = now + OBSERVATION_TTL;
    let decision = policy.observe(
        Observation {
            sequence: 2,
            ..healthy(later)
        },
        later,
    );
    assert_eq!(
        decision.stop.unwrap().reason,
        PolicyReason::StaleObservation
    );
    assert!(!decision.may_run);
}

#[test]
fn duplicate_reordered_and_restamped_samples_cannot_refresh() {
    let now = Instant::now();
    for (sequence, observed_at) in [
        (1, now + Duration::from_secs(4)),
        (0, now + Duration::from_secs(4)),
        (2, now),
    ] {
        let mut policy = running(now);
        policy.observe(
            Observation {
                sequence,
                observed_at,
                ..healthy(now)
            },
            now + Duration::from_secs(4),
        );
        assert!(!policy.evaluate(now + OBSERVATION_TTL).may_run);
    }
}

#[test]
fn urgent_stop_survives_recovery_and_manual_pause() {
    let now = Instant::now();
    let mut policy = running(now);
    let later = now + Duration::from_secs(1);
    policy.observe(
        Observation {
            sequence: 2,
            power: PowerSource::Battery,
            ..healthy(later)
        },
        later,
    );
    let later = later + Duration::from_secs(1);
    let decision = policy.observe(
        Observation {
            sequence: 3,
            memory: MemoryPressure::Critical,
            power: PowerSource::Unknown,
            ..healthy(later)
        },
        later,
    );
    assert_eq!(decision.stop.unwrap().urgency, StopUrgency::Urgent);
    let later = later + Duration::from_secs(1);
    policy.observe(
        Observation {
            sequence: 4,
            ..healthy(later)
        },
        later,
    );
    assert_eq!(
        policy.pause(later).stop.unwrap().urgency,
        StopUrgency::Urgent
    );
    assert!(!policy.request_run(true, later).may_run);
    assert!(!policy.confirm_stopped(later).may_run);
    assert!(policy.request_run(false, later).may_run);
}

#[test]
fn disable_and_terminal_shutdown_dominate_delayed_events() {
    let now = Instant::now();
    let mut policy = running(now);
    policy.disable(now);
    assert_eq!(policy.pause(now).reason, Some(PolicyReason::Disabled));
    policy.confirm_stopped(now);
    assert!(!policy.request_run(false, now).may_run);
    assert!(policy.request_run(true, now).may_run);
    policy.shutdown(now);
    policy.disable(now);
    policy.wake(now);
    policy.observe(
        Observation {
            epoch: policy.epoch(),
            ..healthy(now)
        },
        now,
    );
    policy.confirm_stopped(now);
    assert_eq!(
        policy.request_run(true, now).reason,
        Some(PolicyReason::Shutdown)
    );
}

#[test]
fn wake_rejects_old_epoch_and_requires_new_read_and_resume() {
    let now = Instant::now();
    let mut policy = running(now);
    policy.sleep(now);
    assert!(!policy.observe(healthy(now), now).may_run);
    policy.wake(now);
    policy.confirm_stopped(now);
    policy.observe(
        Observation {
            sequence: 100,
            ..healthy(now)
        },
        now,
    );
    assert_eq!(
        policy.request_run(false, now).reason,
        Some(PolicyReason::MissingObservation)
    );
    assert!(
        !policy
            .observe(
                Observation {
                    epoch: policy.epoch(),
                    ..healthy(now)
                },
                now
            )
            .may_run
    );
    assert!(policy.request_run(false, now).may_run);
}

#[test]
fn invalid_clock_is_latched_until_new_epoch() {
    let now = Instant::now();
    let mut policy = running(now);
    let future = now + Duration::from_secs(1);
    assert_eq!(
        policy
            .observe(
                Observation {
                    sequence: 2,
                    ..healthy(future)
                },
                now
            )
            .stop
            .unwrap()
            .reason,
        PolicyReason::InvalidClock
    );
    policy.confirm_stopped(future);
    assert!(!policy.request_run(false, future).may_run);
    policy.wake(future);
    policy.confirm_stopped(future);
    policy.observe(
        Observation {
            epoch: policy.epoch(),
            ..healthy(future)
        },
        future,
    );
    assert!(policy.request_run(false, future).may_run);
    assert_eq!(
        policy.evaluate(now).stop.unwrap().reason,
        PolicyReason::InvalidClock
    );
}

#[test]
fn missing_information_does_not_queue_start_for_later() {
    let now = Instant::now();
    let mut policy = ResourcePolicy::default();
    assert!(!policy.request_run(true, now).may_run);
    assert!(!policy.observe(healthy(now), now).may_run);
}
