use trading_engine_core::Money;

use crate::RiskError;

const PAPER_CASH_UTILIZATION: f64 = 0.995;

fn check_finite(field: &'static str, value: f64) -> Result<(), RiskError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(RiskError::NonFinite { field, value })
    }
}

/// Computes position size for live/paper execution, matching
/// `paperbot.engine.size_position`:
/// - target units so `(entry - stop) * qty = risk * equity` (`dist = stop_mult * atr`)
/// - capped by `free_quote * 0.995 / (price * (1 + fee))` (spot, no leverage, 0.5% safety buffer)
/// - returns `0.0` when `dist <= 0`, `price <= 0`, `equity <= 0`, or `free_quote <= 0`.
pub fn size_paper_position(
    equity: f64,
    atr: f64,
    stop_mult: f64,
    price: f64,
    free_quote: f64,
    risk: f64,
    fee: f64,
) -> Result<f64, RiskError> {
    check_finite("equity", equity)?;
    check_finite("atr", atr)?;
    check_finite("stop_mult", stop_mult)?;
    check_finite("price", price)?;
    check_finite("free_quote", free_quote)?;
    check_finite("risk", risk)?;
    check_finite("fee", fee)?;

    if !(0.0..1.0).contains(&fee) {
        return Err(RiskError::InvalidFee(fee));
    }
    if risk <= 0.0 || risk > 1.0 {
        return Err(RiskError::InvalidRisk(risk));
    }

    let dist = stop_mult * atr;
    if dist <= 0.0 || price <= 0.0 || equity <= 0.0 || free_quote <= 0.0 {
        return Ok(0.0);
    }

    let risk_qty = risk * equity / dist;
    let cash_cap = free_quote * PAPER_CASH_UTILIZATION / (price * (1.0 + fee));
    Ok(risk_qty.min(cash_cap).max(0.0))
}

/// Computes position size for backtesting, matching `tradebot.backtest.run`:
/// - `q = risk * equity / dist` where `dist = |fill_price - stop_price|`
/// - capped by `max_leverage * equity / (fill_price * (1 + fee))`.
pub fn size_backtest_position(
    equity: f64,
    dist: f64,
    fill_price: f64,
    risk: f64,
    max_leverage: f64,
    fee: f64,
) -> Result<f64, RiskError> {
    check_finite("equity", equity)?;
    check_finite("dist", dist)?;
    check_finite("fill_price", fill_price)?;
    check_finite("risk", risk)?;
    check_finite("max_leverage", max_leverage)?;
    check_finite("fee", fee)?;

    if !(0.0..1.0).contains(&fee) {
        return Err(RiskError::InvalidFee(fee));
    }
    if risk <= 0.0 || risk > 1.0 {
        return Err(RiskError::InvalidRisk(risk));
    }
    if max_leverage <= 0.0 {
        return Err(RiskError::InvalidLeverage(max_leverage));
    }
    if dist <= 0.0 || fill_price <= 0.0 || equity <= 0.0 {
        return Ok(0.0);
    }

    let risk_qty = risk * equity / dist;
    let notional_cap = max_leverage * equity / (fill_price * (1.0 + fee));
    Ok(risk_qty.min(notional_cap).max(0.0))
}

/// Computes the fixed initial stop price for a long position: `entry_price - stop_mult * atr`.
pub fn initial_long_stop(entry_price: Money, atr: f64, stop_mult: f64) -> Result<Money, RiskError> {
    check_finite("atr", atr)?;
    check_finite("stop_mult", stop_mult)?;
    let stop = entry_price.as_f64() - stop_mult * atr;
    Ok(Money::new(stop)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_sizing_risks_one_percent_and_caps_free_cash() {
        // Mirrors `tests/test_paperbot.py::test_sizing_risks_one_percent_and_caps_cash`.
        let q = size_paper_position(10_000.0, 40.0, 2.0, 2_000.0, 10_000.0, 0.01, 0.001).unwrap();
        assert!((q * 2.0 * 40.0 - 100.0).abs() < 1e-12);

        let q = size_paper_position(10_000.0, 1.0, 2.0, 2_000.0, 1_000.0, 0.01, 0.001).unwrap();
        assert!(q * 2_000.0 * 1.001 <= 1_000.0);
        assert!(q > 0.0);

        let q = size_paper_position(10_000.0, 0.0, 2.0, 2_000.0, 10_000.0, 0.01, 0.001).unwrap();
        assert_eq!(q, 0.0);
    }

    #[test]
    fn backtest_sizing_uses_max_leverage_without_paper_buffer() {
        let q = size_backtest_position(10_000.0, 2.0, 2_000.0, 0.01, 1.0, 0.001).unwrap();
        let expected = 10_000.0 / (2_000.0 * 1.001);
        assert!((q - expected).abs() < 1e-12);

        let q_zero = size_backtest_position(10_000.0, 0.0, 2_000.0, 0.01, 1.0, 0.001).unwrap();
        assert_eq!(q_zero, 0.0);
    }

    #[test]
    fn computes_initial_long_stop() {
        let stop = initial_long_stop(Money::new(2_000.0).unwrap(), 40.0, 2.0).unwrap();
        assert_eq!(stop.as_f64(), 1_920.0);
    }

    #[test]
    fn rejects_invalid_sizing_inputs() {
        assert!(matches!(
            size_paper_position(f64::NAN, 40.0, 2.0, 2_000.0, 10_000.0, 0.01, 0.001),
            Err(RiskError::NonFinite {
                field: "equity",
                ..
            })
        ));
        assert!(matches!(
            size_paper_position(10_000.0, 40.0, 2.0, 2_000.0, 10_000.0, 0.0, 0.001),
            Err(RiskError::InvalidRisk(_))
        ));
        assert!(matches!(
            size_backtest_position(10_000.0, 2.0, 2_000.0, 0.01, 0.0, 0.001),
            Err(RiskError::InvalidLeverage(_))
        ));
    }
}
