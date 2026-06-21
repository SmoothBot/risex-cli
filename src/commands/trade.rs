//! Trading commands (JWT path): place/cancel orders, positions, close, etc.
use crate::errors::{Result, RisexError};

const MAX_TICKS: u64 = 16_777_215; // uint24

/// Convert a human size (e.g. 0.01 BTC) into integer `size_steps`.
pub fn size_to_steps(size: f64, step_size: f64) -> Result<u32> {
    if step_size <= 0.0 {
        return Err(RisexError::Validation(
            "market step_size is not positive".into(),
        ));
    }
    let steps = (size / step_size).round();
    if steps < 1.0 {
        return Err(RisexError::Validation(format!(
            "size {size} is below one step ({step_size})"
        )));
    }
    if steps > u32::MAX as f64 {
        return Err(RisexError::Validation("size too large".into()));
    }
    Ok(steps as u32)
}

/// Convert a human price (e.g. 64000.0) into integer `price_ticks` (uint24).
pub fn price_to_ticks(price: f64, step_price: f64) -> Result<u32> {
    if step_price <= 0.0 {
        return Err(RisexError::Validation(
            "market step_price is not positive".into(),
        ));
    }
    let ticks = (price / step_price).round();
    if ticks < 0.0 || ticks as u64 > MAX_TICKS {
        return Err(RisexError::Validation(format!(
            "price {price} out of range for tick {step_price}"
        )));
    }
    Ok(ticks as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_steps_round_to_nearest() {
        assert_eq!(size_to_steps(0.01, 0.000001).unwrap(), 10_000);
        assert_eq!(size_to_steps(0.1, 0.001).unwrap(), 100);
    }

    #[test]
    fn price_ticks_round_to_nearest() {
        assert_eq!(price_to_ticks(64000.0, 0.1).unwrap(), 640_000);
        assert_eq!(price_to_ticks(1700.55, 0.01).unwrap(), 170_055);
    }

    #[test]
    fn price_ticks_overflow_errors() {
        assert!(price_to_ticks(2_000_000.0, 0.1).is_err()); // > uint24
    }

    #[test]
    fn zero_size_errors() {
        assert!(size_to_steps(0.0, 0.001).is_err());
    }
}
