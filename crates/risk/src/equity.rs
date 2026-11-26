use crate::RiskError;

/// Mark-to-market snapshot of one open long position for equity computation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpenPositionMark {
    pub quantity: f64,
    pub entry_price: f64,
    pub mark_price: f64,
}

/// Computes bot equity matching `paperbot.engine.Engine.equity`:
///
/// 1. `raw_equity = quote_balance + sum(pos.quantity * pos.mark_price)`
/// 2. If `capital_cap` is `Some(cap)`:
///    `capped = cap + realized_pnl + sum(pos.quantity * (pos.mark_price - pos.entry_price))`
///    `equity = raw_equity.min(capped)`
pub fn compute_equity(
    quote_balance: f64,
    open_positions: &[OpenPositionMark],
    realized_pnl: f64,
    capital_cap: Option<f64>,
) -> Result<f64, RiskError> {
    if !quote_balance.is_finite() {
        return Err(RiskError::NonFinite {
            field: "quote_balance",
            value: quote_balance,
        });
    }
    if !realized_pnl.is_finite() {
        return Err(RiskError::NonFinite {
            field: "realized_pnl",
            value: realized_pnl,
        });
    }

    let mut raw_equity = quote_balance;
    let mut open_pnl = 0.0;
    for pos in open_positions {
        if !pos.quantity.is_finite() {
            return Err(RiskError::NonFinite {
                field: "quantity",
                value: pos.quantity,
            });
        }
        if !pos.entry_price.is_finite() {
            return Err(RiskError::NonFinite {
                field: "entry_price",
                value: pos.entry_price,
            });
        }
        if !pos.mark_price.is_finite() {
            return Err(RiskError::NonFinite {
                field: "mark_price",
                value: pos.mark_price,
            });
        }
        raw_equity += pos.quantity * pos.mark_price;
        open_pnl += pos.quantity * (pos.mark_price - pos.entry_price);
    }

    if let Some(cap) = capital_cap {
        if !cap.is_finite() {
            return Err(RiskError::NonFinite {
                field: "capital_cap",
                value: cap,
            });
        }
        if cap <= 0.0 {
            return Err(RiskError::InvalidCapitalCap(cap));
        }
        Ok(raw_equity.min(cap + realized_pnl + open_pnl))
    } else {
        Ok(raw_equity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_uncapped_and_capped_equity() {
        let positions = [OpenPositionMark {
            quantity: 2.0,
            entry_price: 2_000.0,
            mark_price: 2_100.0,
        }];
        // Huge testnet balance of 500_000 USDT + 2 ETH @ 2100 = 504_200 raw equity.
        let uncapped = compute_equity(500_000.0, &positions, 150.0, None).unwrap();
        assert_eq!(uncapped, 504_200.0);

        // Capped at 10_000 + 150 realized + 2*(2100-2000)=200 open = 10_350.
        let capped = compute_equity(500_000.0, &positions, 150.0, Some(10_000.0)).unwrap();
        assert_eq!(capped, 10_350.0);

        // If actual account balance is lower than cap + PnL, raw_equity wins.
        let low_bal = compute_equity(5_000.0, &positions, 150.0, Some(10_000.0)).unwrap();
        assert_eq!(low_bal, 9_200.0);
    }

    #[test]
    fn rejects_invalid_capital_cap_or_non_finite() {
        assert!(matches!(
            compute_equity(10_000.0, &[], 0.0, Some(0.0)),
            Err(RiskError::InvalidCapitalCap(_))
        ));
        assert!(matches!(
            compute_equity(f64::NAN, &[], 0.0, None),
            Err(RiskError::NonFinite {
                field: "quote_balance",
                ..
            })
        ));
    }
}
