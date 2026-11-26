use chrono::NaiveDate;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::RiskError;

mod opt_date {
    use super::*;

    pub fn serialize<S>(date: &Option<NaiveDate>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match date {
            Some(d) => serializer.serialize_some(&d.format("%Y-%m-%d").to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<NaiveDate>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt = Option::<String>::deserialize(deserializer)?;
        opt.map(|s| {
            NaiveDate::parse_from_str(&s, "%Y-%m-%d")
                .map_err(|err| serde::de::Error::custom(err.to_string()))
        })
        .transpose()
    }
}

/// Persisted daily risk-gate bookkeeping matching `paperbot/state.py` fields
/// `day`, `day_start_equity`, and `paused_day`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DailyRiskState {
    #[serde(with = "opt_date")]
    pub day: Option<NaiveDate>,
    pub day_start_equity: Option<f64>,
    #[serde(with = "opt_date")]
    pub paused_day: Option<NaiveDate>,
}

/// Reason why new entries are blocked by the risk gate.
#[derive(Debug, Clone, PartialEq)]
pub enum RiskBlockReason {
    KillSwitch,
    PauseFile,
    DailyLossCap {
        equity: f64,
        threshold: f64,
        day_start_equity: f64,
    },
}

impl RiskBlockReason {
    /// Short label matching Python's `risk_gate` return values.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::KillSwitch => "kill switch",
            Self::PauseFile => "pause file",
            Self::DailyLossCap { .. } => "daily loss cap",
        }
    }
}

/// Outcome of evaluating [`evaluate_risk_gate`].
#[derive(Debug, Clone, PartialEq)]
pub struct RiskGateOutcome {
    pub blocked: Option<RiskBlockReason>,
    pub day_rolled: bool,
    pub daily_cap_triggered: bool,
}

/// Evaluates the daily risk gate and rolls the UTC day when needed, matching
/// `paperbot.engine.Engine.risk_gate`.
pub fn evaluate_risk_gate(
    state: &mut DailyRiskState,
    today_utc: NaiveDate,
    equity: f64,
    daily_loss_cap: f64,
    kill_active: bool,
    pause_active: bool,
) -> Result<RiskGateOutcome, RiskError> {
    if !equity.is_finite() {
        return Err(RiskError::NonFinite {
            field: "equity",
            value: equity,
        });
    }
    if !daily_loss_cap.is_finite() {
        return Err(RiskError::NonFinite {
            field: "daily_loss_cap",
            value: daily_loss_cap,
        });
    }
    if daily_loss_cap <= 0.0 || daily_loss_cap > 1.0 {
        return Err(RiskError::InvalidDailyLossCap(daily_loss_cap));
    }

    let mut day_rolled = false;
    if state.day != Some(today_utc) {
        state.day = Some(today_utc);
        state.day_start_equity = Some(equity);
        state.paused_day = None;
        day_rolled = true;
    }

    if kill_active {
        return Ok(RiskGateOutcome {
            blocked: Some(RiskBlockReason::KillSwitch),
            day_rolled,
            daily_cap_triggered: false,
        });
    }

    if pause_active {
        return Ok(RiskGateOutcome {
            blocked: Some(RiskBlockReason::PauseFile),
            day_rolled,
            daily_cap_triggered: false,
        });
    }

    let start = match state.day_start_equity {
        Some(value) if value != 0.0 => value,
        _ => equity,
    };
    let threshold = start * (1.0 - daily_loss_cap);

    if state.paused_day == Some(today_utc) {
        return Ok(RiskGateOutcome {
            blocked: Some(RiskBlockReason::DailyLossCap {
                equity,
                threshold,
                day_start_equity: start,
            }),
            day_rolled,
            daily_cap_triggered: false,
        });
    }

    if equity <= threshold {
        state.paused_day = Some(today_utc);
        return Ok(RiskGateOutcome {
            blocked: Some(RiskBlockReason::DailyLossCap {
                equity,
                threshold,
                day_start_equity: start,
            }),
            day_rolled,
            daily_cap_triggered: true,
        });
    }

    Ok(RiskGateOutcome {
        blocked: None,
        day_rolled,
        daily_cap_triggered: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn rolls_utc_day_and_clears_previous_daily_pause() {
        let mut state = DailyRiskState {
            day: Some(date(2024, 1, 1)),
            day_start_equity: Some(10_000.0),
            paused_day: Some(date(2024, 1, 1)),
        };

        let out =
            evaluate_risk_gate(&mut state, date(2024, 1, 2), 9_600.0, 0.03, false, false).unwrap();

        assert_eq!(out.blocked, None);
        assert!(out.day_rolled);
        assert!(!out.daily_cap_triggered);
        assert_eq!(state.day, Some(date(2024, 1, 2)));
        assert_eq!(state.day_start_equity, Some(9_600.0));
        assert_eq!(state.paused_day, None);
    }

    #[test]
    fn triggers_and_persists_daily_loss_cap() {
        // Mirrors `tests/test_paperbot.py::test_daily_loss_cap_blocks_entries`.
        let today = date(2024, 1, 1);
        let mut state = DailyRiskState {
            day: Some(today),
            day_start_equity: Some(10_400.0),
            paused_day: None,
        };

        let out = evaluate_risk_gate(&mut state, today, 10_000.0, 0.03, false, false).unwrap();
        assert!(matches!(
            out.blocked,
            Some(RiskBlockReason::DailyLossCap { .. })
        ));
        assert!(out.daily_cap_triggered);
        assert_eq!(state.paused_day, Some(today));

        // Subsequent check on the same day stays blocked even if equity recovers.
        let out2 = evaluate_risk_gate(&mut state, today, 10_300.0, 0.03, false, false).unwrap();
        assert!(matches!(
            out2.blocked,
            Some(RiskBlockReason::DailyLossCap { .. })
        ));
        assert!(!out2.daily_cap_triggered);
    }

    #[test]
    fn prioritizes_kill_and_pause_over_daily_cap() {
        let today = date(2024, 1, 1);
        let mut state = DailyRiskState::default();

        let kill = evaluate_risk_gate(&mut state, today, 10_000.0, 0.03, true, true).unwrap();
        assert_eq!(kill.blocked, Some(RiskBlockReason::KillSwitch));
        assert!(kill.day_rolled);

        let pause = evaluate_risk_gate(&mut state, today, 10_000.0, 0.03, false, true).unwrap();
        assert_eq!(pause.blocked, Some(RiskBlockReason::PauseFile));
        assert!(!pause.day_rolled);
    }
}
