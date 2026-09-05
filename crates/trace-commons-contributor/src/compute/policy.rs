//! Pure resource gate; not yet connected to the worker actor or native observers.
//!
//! This is an additional launch condition, never consent, artifact verification,
//! or proof of process termination. The owning actor must serialize commands,
//! samples and timer ticks, poll even without samples, and recheck before spawn.
//! A stop stays latched until the actor confirms its owned child has been reaped.

use std::time::{Duration, Instant};

pub const OBSERVATION_TTL: Duration = Duration::from_secs(6);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerSource {
    Ac,
    Battery,
    Ups,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalState {
    Nominal,
    Fair,
    Serious,
    Critical,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryPressure {
    Normal,
    Warning,
    Critical,
    Unknown,
}

/// One complete fresh platform read. Never stamp cached fields with a new time;
/// unknown/unsupported reads must remain unknown. Sequence increases per epoch.
#[derive(Debug, Clone, Copy)]
pub struct Observation {
    pub epoch: u64,
    pub sequence: u64,
    pub observed_at: Instant,
    pub power: PowerSource,
    pub low_power_mode: Option<bool>,
    pub thermal: ThermalState,
    pub memory: MemoryPressure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StopUrgency {
    Normal,
    Urgent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyReason {
    MissingObservation,
    StaleObservation,
    InvalidClock,
    Sleeping,
    Power,
    LowPowerMode,
    UnknownObservation,
    Thermal,
    CriticalThermal,
    Memory,
    CriticalMemory,
    Paused,
    Disabled,
    Shutdown,
}

impl PolicyReason {
    pub fn detail(self) -> &'static str {
        match self {
            Self::MissingObservation => "Waiting for fresh resource observations.",
            Self::StaleObservation => "Resource observations expired. Resume after they recover.",
            Self::InvalidClock => "Resource observation timing is invalid.",
            Self::Sleeping => "Compute is paused for sleep. Resume after waking.",
            Self::Power => "Compute requires AC power. Resume after reconnecting.",
            Self::LowPowerMode => "Turn off Low Power Mode before resuming compute.",
            Self::UnknownObservation => "Required resource information is unavailable.",
            Self::Thermal | Self::CriticalThermal => {
                "Compute is paused until the device cools. Resume when ready."
            }
            Self::Memory | Self::CriticalMemory => {
                "Compute is paused for memory pressure. Resume after recovery."
            }
            Self::Paused => "Compute is paused. Resume when ready.",
            Self::Disabled => "Compute contribution is disabled.",
            Self::Shutdown => "Compute is shutting down.",
        }
    }

    fn urgency(self) -> StopUrgency {
        match self {
            Self::CriticalThermal | Self::CriticalMemory => StopUrgency::Urgent,
            _ => StopUrgency::Normal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopRequest {
    pub reason: PolicyReason,
    pub urgency: StopUrgency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub may_run: bool,
    pub reason: Option<PolicyReason>,
    pub stop: Option<StopRequest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intent {
    Paused,
    Requested,
    Disabled,
    Shutdown,
}

#[derive(Debug)]
pub struct ResourcePolicy {
    epoch: u64,
    sample: Option<Observation>,
    last_now: Option<Instant>,
    clock_invalid: bool,
    sleeping: bool,
    intent: Intent,
    stop: Option<StopRequest>,
}

impl Default for ResourcePolicy {
    fn default() -> Self {
        Self {
            epoch: 0,
            sample: None,
            last_now: None,
            clock_invalid: false,
            sleeping: false,
            intent: Intent::Paused,
            stop: None,
        }
    }
}

impl ResourcePolicy {
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Old epochs, duplicate sequences and reordered capture times cannot refresh
    /// the lease. Future timestamps invalidate the gate until wake/new session.
    pub fn observe(&mut self, sample: Observation, now: Instant) -> Decision {
        // Expiry must latch before a new healthy sample can replace an old one.
        self.evaluate(now);
        if sample.epoch == self.epoch && !self.sleeping {
            if sample.observed_at > now {
                self.clock_invalid = true;
            } else if self.sample.is_none_or(|old| {
                sample.sequence > old.sequence && sample.observed_at > old.observed_at
            }) {
                self.sample = Some(sample);
            }
        }
        self.evaluate(now)
    }

    /// Called only following the host's explicit Enable/Resume journey. The host
    /// still owns consent persistence and all other launch gates. A disabled gate
    /// requires Enable, not Resume. A failed attempt is not queued for recovery.
    pub fn request_run(&mut self, enable: bool, now: Instant) -> Decision {
        self.evaluate(now);
        if self.intent != Intent::Shutdown
            && (enable || self.intent != Intent::Disabled)
            && self.stop.is_none()
            && self.resource_reason(now).is_none()
        {
            self.intent = Intent::Requested;
        }
        self.evaluate(now)
    }

    pub fn pause(&mut self, now: Instant) -> Decision {
        if self.intent != Intent::Shutdown && self.intent != Intent::Disabled {
            self.intent = Intent::Paused;
        }
        self.latch(PolicyReason::Paused);
        self.evaluate(now)
    }

    pub fn disable(&mut self, now: Instant) -> Decision {
        if self.intent != Intent::Shutdown {
            self.intent = Intent::Disabled;
        }
        self.latch(PolicyReason::Disabled);
        self.evaluate(now)
    }

    pub fn shutdown(&mut self, now: Instant) -> Decision {
        self.intent = Intent::Shutdown;
        self.latch(PolicyReason::Shutdown);
        self.evaluate(now)
    }

    pub fn sleep(&mut self, now: Instant) -> Decision {
        self.sleeping = true;
        self.sample = None;
        self.latch(PolicyReason::Sleeping);
        self.evaluate(now)
    }

    /// The adapter must discard cached readings and use the new epoch. The actor
    /// must separately invalidate worker telemetry and reconcile child ownership.
    pub fn wake(&mut self, now: Instant) -> Decision {
        self.sleeping = false;
        self.sample = None;
        self.clock_invalid = false;
        self.last_now = None;
        if let Some(epoch) = self.epoch.checked_add(1) {
            self.epoch = epoch;
        } else {
            self.intent = Intent::Shutdown;
        }
        if self.intent == Intent::Requested {
            self.intent = Intent::Paused;
        }
        self.latch(PolicyReason::MissingObservation);
        self.evaluate(now)
    }

    /// Not a drain acknowledgment: call only after reaping, or proving no owned
    /// child exists. Failed stop/reap must leave this latch untouched.
    pub fn confirm_stopped(&mut self, now: Instant) -> Decision {
        self.stop = None;
        if self.intent == Intent::Requested {
            self.intent = Intent::Paused;
        }
        self.evaluate(now)
    }

    pub fn evaluate(&mut self, now: Instant) -> Decision {
        if self.last_now.is_some_and(|last| now < last) {
            self.clock_invalid = true;
        }
        self.last_now = Some(now);
        let resource = self.resource_reason(now);
        if let Some(reason) = resource {
            if self.intent == Intent::Requested || self.stop.is_some() {
                self.latch(reason);
            }
            if self.intent == Intent::Requested {
                self.intent = Intent::Paused;
            }
        }
        let reason = match self.intent {
            Intent::Shutdown => Some(PolicyReason::Shutdown),
            Intent::Disabled => Some(PolicyReason::Disabled),
            _ => self
                .stop
                .map(|stop| stop.reason)
                .or(resource)
                .or((self.intent == Intent::Paused).then_some(PolicyReason::Paused)),
        };
        Decision {
            may_run: reason.is_none(),
            reason,
            stop: self.stop,
        }
    }

    fn latch(&mut self, reason: PolicyReason) {
        let request = StopRequest {
            reason,
            urgency: reason.urgency(),
        };
        if self.stop.is_none_or(|old| request.urgency > old.urgency) {
            self.stop = Some(request);
        }
    }

    fn resource_reason(&self, now: Instant) -> Option<PolicyReason> {
        if self.sleeping {
            return Some(PolicyReason::Sleeping);
        }
        if self.clock_invalid {
            return Some(PolicyReason::InvalidClock);
        }
        let Some(sample) = self.sample else {
            return Some(PolicyReason::MissingObservation);
        };
        // Critical readings must escalate even when another field is unknown.
        if sample.thermal == ThermalState::Critical {
            return Some(PolicyReason::CriticalThermal);
        }
        if sample.memory == MemoryPressure::Critical {
            return Some(PolicyReason::CriticalMemory);
        }
        let Some(age) = now.checked_duration_since(sample.observed_at) else {
            return Some(PolicyReason::InvalidClock);
        };
        if age >= OBSERVATION_TTL {
            return Some(PolicyReason::StaleObservation);
        }
        if sample.power == PowerSource::Unknown
            || sample.low_power_mode.is_none()
            || sample.thermal == ThermalState::Unknown
            || sample.memory == MemoryPressure::Unknown
        {
            return Some(PolicyReason::UnknownObservation);
        }
        if sample.power != PowerSource::Ac {
            return Some(PolicyReason::Power);
        }
        if sample.low_power_mode == Some(true) {
            return Some(PolicyReason::LowPowerMode);
        }
        if sample.thermal != ThermalState::Nominal {
            return Some(PolicyReason::Thermal);
        }
        if sample.memory != MemoryPressure::Normal {
            return Some(PolicyReason::Memory);
        }
        None
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
