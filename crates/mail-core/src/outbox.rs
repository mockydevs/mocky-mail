//! Outbox rules: when a queued message may be sent, how retries back off,
//! and how a crash mid-send is recovered without sending twice.
//!
//! The state machine is deliberately strict. Every transition that is not
//! listed is refused with a [`TransitionError`], so a bug elsewhere shows up
//! as an error instead of a duplicate or lost email.
//!
//! ```text
//!            begin_send               Delivered
//!  Queued ─────────────▶ Sending ─────────────▶ Sent
//!    ▲  ▲                  │  │
//!    │  └── Transient ─────┘  └── Permanent / gave up ──▶ Failed
//!    │      (backoff)                                       │
//!    └──────────────────── send_now ────────────────────────┘
//!  Queued, Failed ── cancel ──▶ Cancelled
//!  Sending ── app restarted ──▶ Queued (maybe_sent: check before resending)
//!  Queued (maybe_sent) ── can't check ──▶ Failed (unconfirmed: user decides)
//! ```

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::limits::TooLarge;

/// Seconds since the Unix epoch. The caller supplies "now".
pub type UnixSeconds = u64;

/// How long to wait between automatic retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Delay after the first failed attempt, in seconds.
    pub base_delay: u64,
    /// Longest delay between attempts, in seconds.
    pub max_delay: u64,
    /// Attempts before giving up and marking the message failed.
    pub max_attempts: u32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            base_delay: 10,
            max_delay: 15 * 60,
            max_attempts: 8,
        }
    }
}

impl RetryPolicy {
    /// Delay before the next attempt after `attempts` failed attempts.
    /// Doubles each time, capped at `max_delay`.
    #[must_use]
    pub fn delay_after(&self, attempts: u32) -> u64 {
        let exponent = attempts.saturating_sub(1).min(63);
        let factor = 1u64.checked_shl(exponent).unwrap_or(u64::MAX);
        self.base_delay.saturating_mul(factor).min(self.max_delay)
    }
}

/// Why a message ended up in the failed state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    /// The server refused the message permanently (bad address, policy).
    Rejected,
    /// Temporary errors kept happening until retries ran out.
    GaveUp,
    /// The message is bigger than the server accepts.
    TooLarge,
    /// The app stopped mid-send and there was no way to check whether the
    /// server got it (no Sent folder to look in, or its saved state was
    /// damaged). The user decides whether to send it again.
    Unconfirmed,
}

/// Where a message is in the outbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum OutboxState {
    /// Waiting to be sent at or after `next_attempt_at`.
    Queued {
        /// Earliest time the next attempt may start.
        next_attempt_at: UnixSeconds,
        /// The app stopped while this message was being sent, so the server
        /// may already have it. It must be checked before sending again.
        maybe_sent: bool,
    },
    /// Being handed to the server right now.
    Sending {
        /// When this attempt started.
        started_at: UnixSeconds,
    },
    /// Accepted by the server. Final.
    Sent {
        /// When the server accepted it.
        at: UnixSeconds,
    },
    /// Needs the user: edit, retry or delete.
    Failed {
        /// Category of failure, for the UI.
        reason: FailureReason,
        /// Detail from the server or the size check.
        message: String,
    },
    /// The user cancelled it. Final.
    Cancelled,
}

impl OutboxState {
    /// Short name used in errors and logs.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Queued { .. } => "queued",
            Self::Sending { .. } => "sending",
            Self::Sent { .. } => "sent",
            Self::Failed { .. } => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Result of one attempt to hand a message to the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// The server accepted the message.
    Delivered,
    /// A temporary problem (network dropped, 4xx reply). Worth retrying.
    Transient(String),
    /// A permanent refusal (5xx reply). Retrying will not help.
    Permanent(String),
}

/// A transition that is not allowed from the current state.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransitionError {
    /// The action is not valid in this state.
    #[error("cannot {action} a message that is {from}")]
    NotAllowed {
        /// What was attempted.
        action: &'static str,
        /// The state it was attempted from.
        from: &'static str,
    },
    /// The message is not due yet.
    #[error("message is not due until {next_attempt_at}")]
    NotDue {
        /// When it becomes due.
        next_attempt_at: UnixSeconds,
    },
    /// The app restarted mid-send. The caller must check whether the server
    /// already has the message (by Message-ID) before sending again.
    #[error("message may already have been sent; check before resending")]
    DuplicateCheckRequired,
}

/// One message in the outbox and its delivery state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxItem {
    /// Attempts made since it was queued (or last retried by the user).
    pub attempts: u32,
    /// Current state.
    pub state: OutboxState,
}

impl OutboxItem {
    /// A newly queued message, due immediately.
    #[must_use]
    pub const fn new(now: UnixSeconds) -> Self {
        Self {
            attempts: 0,
            state: OutboxState::Queued {
                next_attempt_at: now,
                maybe_sent: false,
            },
        }
    }

    fn refuse(&self, action: &'static str) -> TransitionError {
        TransitionError::NotAllowed {
            action,
            from: self.state.name(),
        }
    }

    /// Whether the message should be picked up by the sender now.
    #[must_use]
    pub const fn is_due(&self, now: UnixSeconds) -> bool {
        matches!(self.state, OutboxState::Queued { next_attempt_at, .. } if next_attempt_at <= now)
    }

    /// Start an attempt.
    ///
    /// # Errors
    /// Refused unless the message is queued, due, and not awaiting a
    /// duplicate check.
    pub fn begin_send(&mut self, now: UnixSeconds) -> Result<(), TransitionError> {
        match self.state {
            OutboxState::Queued {
                maybe_sent: true, ..
            } => Err(TransitionError::DuplicateCheckRequired),
            OutboxState::Queued {
                next_attempt_at, ..
            } if next_attempt_at > now => Err(TransitionError::NotDue { next_attempt_at }),
            OutboxState::Queued { .. } => {
                self.attempts = self.attempts.saturating_add(1);
                self.state = OutboxState::Sending { started_at: now };
                Ok(())
            }
            _ => Err(self.refuse("send")),
        }
    }

    /// Record the result of the attempt started by [`Self::begin_send`].
    ///
    /// # Errors
    /// Refused unless the message is currently sending.
    pub fn finish(
        &mut self,
        outcome: SendOutcome,
        now: UnixSeconds,
        policy: &RetryPolicy,
    ) -> Result<(), TransitionError> {
        if !matches!(self.state, OutboxState::Sending { .. }) {
            return Err(self.refuse("finish"));
        }
        self.state = match outcome {
            SendOutcome::Delivered => OutboxState::Sent { at: now },
            SendOutcome::Permanent(message) => OutboxState::Failed {
                reason: FailureReason::Rejected,
                message,
            },
            SendOutcome::Transient(message) if self.attempts >= policy.max_attempts => {
                OutboxState::Failed {
                    reason: FailureReason::GaveUp,
                    message,
                }
            }
            SendOutcome::Transient(_) => OutboxState::Queued {
                next_attempt_at: now.saturating_add(policy.delay_after(self.attempts)),
                maybe_sent: false,
            },
        };
        Ok(())
    }

    /// Mark a queued message as too large to send, before any attempt.
    ///
    /// # Errors
    /// Refused unless the message is queued.
    pub fn fail_too_large(&mut self, error: TooLarge) -> Result<(), TransitionError> {
        if !matches!(self.state, OutboxState::Queued { .. }) {
            return Err(self.refuse("size-check"));
        }
        self.state = OutboxState::Failed {
            reason: FailureReason::TooLarge,
            message: error.to_string(),
        };
        Ok(())
    }

    /// The user pressed "Send now" or "Retry".
    ///
    /// # Errors
    /// Refused while sending, or once sent or cancelled.
    pub fn send_now(&mut self, now: UnixSeconds) -> Result<(), TransitionError> {
        match &mut self.state {
            OutboxState::Queued {
                next_attempt_at, ..
            } => {
                *next_attempt_at = now;
                Ok(())
            }
            OutboxState::Failed { .. } => {
                self.attempts = 0;
                self.state = OutboxState::Queued {
                    next_attempt_at: now,
                    maybe_sent: false,
                };
                Ok(())
            }
            _ => Err(self.refuse("retry")),
        }
    }

    /// The user cancelled the message.
    ///
    /// # Errors
    /// Refused while sending (it may reach the server regardless), and once
    /// sent or cancelled.
    pub fn cancel(&mut self) -> Result<(), TransitionError> {
        match self.state {
            OutboxState::Queued { .. } | OutboxState::Failed { .. } => {
                self.state = OutboxState::Cancelled;
                Ok(())
            }
            _ => Err(self.refuse("cancel")),
        }
    }

    /// Call for every item when the app starts. A message left in
    /// `Sending` was interrupted; it goes back to the queue flagged
    /// `maybe_sent`. The interrupted attempt is not counted against the
    /// retry limit. Returns whether anything changed.
    pub fn recover_after_restart(&mut self, now: UnixSeconds) -> bool {
        if matches!(self.state, OutboxState::Sending { .. }) {
            self.attempts = self.attempts.saturating_sub(1);
            self.state = OutboxState::Queued {
                next_attempt_at: now,
                maybe_sent: true,
            };
            true
        } else {
            false
        }
    }

    /// A stand-in for an entry whose saved state can't be read. Nobody knows
    /// whether it was sent, so it waits for the user instead of sending.
    #[must_use]
    pub fn damaged() -> Self {
        Self {
            attempts: 0,
            state: OutboxState::Failed {
                reason: FailureReason::Unconfirmed,
                message: "This message's saved delivery state was damaged, so it may or may not have been sent.".to_owned(),
            },
        }
    }

    /// The duplicate check can't be done (no Sent folder, no Message-ID, the
    /// server won't open the folder). Rather than guess, stop and ask the
    /// user: sending again might deliver it twice.
    ///
    /// # Errors
    /// Refused unless the message is queued and awaiting that check.
    pub fn cannot_confirm(&mut self, message: String) -> Result<(), TransitionError> {
        match self.state {
            OutboxState::Queued {
                maybe_sent: true, ..
            } => {
                self.state = OutboxState::Failed {
                    reason: FailureReason::Unconfirmed,
                    message,
                };
                Ok(())
            }
            _ => Err(self.refuse("give up checking")),
        }
    }

    /// Result of checking the server's Sent folder for this message.
    ///
    /// # Errors
    /// Refused unless the message is queued and awaiting that check.
    pub fn resolve_duplicate_check(
        &mut self,
        found_on_server: bool,
        now: UnixSeconds,
    ) -> Result<(), TransitionError> {
        match self.state {
            OutboxState::Queued {
                maybe_sent: true, ..
            } => {
                self.state = if found_on_server {
                    OutboxState::Sent { at: now }
                } else {
                    OutboxState::Queued {
                        next_attempt_at: now,
                        maybe_sent: false,
                    }
                };
                Ok(())
            }
            _ => Err(self.refuse("resolve duplicate check for")),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::arithmetic_side_effects, clippy::panic)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const T0: UnixSeconds = 1_700_000_000;

    fn sending(now: UnixSeconds) -> OutboxItem {
        let mut item = OutboxItem::new(now);
        item.begin_send(now).unwrap();
        item
    }

    #[test]
    fn new_item_is_due_immediately() {
        let item = OutboxItem::new(T0);
        assert!(item.is_due(T0));
        assert!(!item.is_due(T0 - 1));
    }

    #[test]
    fn delivered_message_is_sent_and_final() {
        let policy = RetryPolicy::default();
        let mut item = sending(T0);
        item.finish(SendOutcome::Delivered, T0 + 1, &policy)
            .unwrap();
        assert_eq!(item.state, OutboxState::Sent { at: T0 + 1 });
        assert!(item.begin_send(T0 + 2).is_err());
        assert!(item.cancel().is_err());
        assert!(item.send_now(T0 + 2).is_err());
        assert!(!item.recover_after_restart(T0 + 2));
    }

    #[test]
    fn transient_failure_backs_off_then_gives_up() {
        let policy = RetryPolicy {
            base_delay: 10,
            max_delay: 60,
            max_attempts: 3,
        };
        let mut item = OutboxItem::new(T0);
        let mut now = T0;
        for expected_delay in [10, 20] {
            item.begin_send(now).unwrap();
            item.finish(SendOutcome::Transient("timeout".into()), now, &policy)
                .unwrap();
            let OutboxState::Queued {
                next_attempt_at, ..
            } = item.state
            else {
                panic!("expected queued, got {:?}", item.state)
            };
            assert_eq!(next_attempt_at, now + expected_delay);
            assert_eq!(
                item.begin_send(now),
                Err(TransitionError::NotDue { next_attempt_at })
            );
            now = next_attempt_at;
        }
        item.begin_send(now).unwrap();
        item.finish(SendOutcome::Transient("timeout".into()), now, &policy)
            .unwrap();
        assert_eq!(
            item.state,
            OutboxState::Failed {
                reason: FailureReason::GaveUp,
                message: "timeout".into()
            }
        );
    }

    #[test]
    fn permanent_failure_does_not_retry() {
        let mut item = sending(T0);
        item.finish(
            SendOutcome::Permanent("550 no such user".into()),
            T0,
            &RetryPolicy::default(),
        )
        .unwrap();
        assert!(matches!(
            item.state,
            OutboxState::Failed {
                reason: FailureReason::Rejected,
                ..
            }
        ));
        assert!(!item.is_due(T0 + 1_000_000));
    }

    #[test]
    fn user_retry_after_failure_resets_attempts() {
        let mut item = sending(T0);
        item.finish(
            SendOutcome::Permanent("552 too big".into()),
            T0,
            &RetryPolicy::default(),
        )
        .unwrap();
        item.send_now(T0 + 5).unwrap();
        assert_eq!(item.attempts, 0);
        assert!(item.is_due(T0 + 5));
    }

    #[test]
    fn send_now_skips_the_backoff_wait() {
        let mut item = sending(T0);
        item.finish(
            SendOutcome::Transient("dropped".into()),
            T0,
            &RetryPolicy::default(),
        )
        .unwrap();
        assert!(!item.is_due(T0 + 1));
        item.send_now(T0 + 1).unwrap();
        assert!(item.is_due(T0 + 1));
    }

    #[test]
    fn cannot_cancel_while_sending() {
        let mut item = sending(T0);
        assert_eq!(
            item.cancel(),
            Err(TransitionError::NotAllowed {
                action: "cancel",
                from: "sending"
            })
        );
    }

    #[test]
    fn finish_without_begin_is_refused() {
        let mut item = OutboxItem::new(T0);
        assert!(
            item.finish(SendOutcome::Delivered, T0, &RetryPolicy::default())
                .is_err()
        );
        assert_eq!(item, OutboxItem::new(T0));
    }

    #[test]
    fn crash_mid_send_requires_duplicate_check() {
        let mut item = sending(T0);
        assert!(item.recover_after_restart(T0 + 60));
        assert_eq!(
            item.begin_send(T0 + 60),
            Err(TransitionError::DuplicateCheckRequired)
        );

        let mut found = item.clone();
        found.resolve_duplicate_check(true, T0 + 61).unwrap();
        assert_eq!(found.state, OutboxState::Sent { at: T0 + 61 });

        item.resolve_duplicate_check(false, T0 + 61).unwrap();
        item.begin_send(T0 + 61).unwrap();
    }

    #[test]
    fn too_large_fails_before_any_attempt() {
        let mut item = OutboxItem::new(T0);
        item.fail_too_large(TooLarge {
            estimated: 30,
            limit: 25,
        })
        .unwrap();
        assert_eq!(item.attempts, 0);
        assert!(matches!(
            item.state,
            OutboxState::Failed {
                reason: FailureReason::TooLarge,
                ..
            }
        ));
    }

    #[test]
    fn an_uncheckable_message_waits_for_the_user() {
        let mut item = sending(T0);
        assert!(item.recover_after_restart(T0 + 5));
        item.cannot_confirm("no Sent folder".into()).unwrap();
        assert!(matches!(
            item.state,
            OutboxState::Failed {
                reason: FailureReason::Unconfirmed,
                ..
            }
        ));
        assert!(!item.is_due(T0 + 1_000));
        // Retry is the user's explicit choice to send it again.
        item.send_now(T0 + 10).unwrap();
        assert!(item.begin_send(T0 + 10).is_ok());
        // Only a message awaiting the check can give up on it.
        assert!(OutboxItem::new(T0).cannot_confirm("x".into()).is_err());
    }

    #[test]
    fn a_damaged_entry_never_sends_by_itself() {
        let item = OutboxItem::damaged();
        assert!(!item.is_due(u64::MAX));
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains(r#""reason":"unconfirmed""#));
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let policy = RetryPolicy {
            base_delay: 10,
            max_delay: 100,
            max_attempts: 99,
        };
        assert_eq!(policy.delay_after(0), 10);
        assert_eq!(policy.delay_after(1), 10);
        assert_eq!(policy.delay_after(2), 20);
        assert_eq!(policy.delay_after(4), 80);
        assert_eq!(policy.delay_after(5), 100);
        assert_eq!(policy.delay_after(u32::MAX), 100);
    }

    #[test]
    fn state_round_trips_through_json() {
        let item = OutboxItem {
            attempts: 2,
            state: OutboxState::Failed {
                reason: FailureReason::TooLarge,
                message: "too big".into(),
            },
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains(r#""state":"failed""#));
        assert!(json.contains(r#""reason":"too_large""#));
        let back: OutboxItem = serde_json::from_str(&json).unwrap();
        assert_eq!(back, item);
    }

    #[derive(Debug, Clone)]
    enum Op {
        Begin,
        Deliver,
        Transient,
        Permanent,
        SendNow,
        Cancel,
        Restart,
        Resolve(bool),
        Wait(u64),
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            Just(Op::Begin),
            Just(Op::Deliver),
            Just(Op::Transient),
            Just(Op::Permanent),
            Just(Op::SendNow),
            Just(Op::Cancel),
            Just(Op::Restart),
            any::<bool>().prop_map(Op::Resolve),
            (0u64..2_000).prop_map(Op::Wait),
        ]
    }

    proptest! {
        /// Whatever happens, a message is accepted by the server at most
        /// once, never sent after cancel, and Sent is final.
        #[test]
        fn never_delivered_twice_and_final_states_hold(ops in proptest::collection::vec(op(), 0..60)) {
            let policy = RetryPolicy { base_delay: 5, max_delay: 300, max_attempts: 4 };
            let mut now = T0;
            let mut item = OutboxItem::new(now);
            let mut deliveries = 0u32;
            let mut attempts_since_reset = 0u32;

            for op in ops {
                let before = item.clone();
                match op {
                    Op::Begin => if item.begin_send(now).is_ok() { attempts_since_reset += 1; },
                    Op::Deliver => if item.finish(SendOutcome::Delivered, now, &policy).is_ok() { deliveries += 1; },
                    Op::Transient => { let _ = item.finish(SendOutcome::Transient("t".into()), now, &policy); },
                    Op::Permanent => { let _ = item.finish(SendOutcome::Permanent("p".into()), now, &policy); },
                    Op::SendNow => if item.send_now(now).is_ok() && matches!(before.state, OutboxState::Failed { .. }) { attempts_since_reset = 0; },
                    Op::Cancel => { let _ = item.cancel(); },
                    Op::Restart => if item.recover_after_restart(now) { attempts_since_reset -= 1; },
                    Op::Resolve(found) => if item.resolve_duplicate_check(found, now).is_ok() && found { deliveries += 1; },
                    Op::Wait(secs) => now += secs,
                }

                if matches!(before.state, OutboxState::Sent { .. } | OutboxState::Cancelled) {
                    prop_assert_eq!(&item, &before, "final state changed");
                }
                prop_assert!(deliveries <= 1);
                prop_assert_eq!(item.attempts, attempts_since_reset);
                prop_assert!(item.attempts <= policy.max_attempts);
            }
        }
    }
}
