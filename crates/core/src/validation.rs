use crate::error::DomainError;

pub(crate) fn ensure_finite(field: &'static str, value: f64) -> Result<(), DomainError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(DomainError::NonFinite { field, value })
    }
}

pub(crate) fn ensure_positive_price(field: &'static str, value: f64) -> Result<(), DomainError> {
    ensure_finite(field, value)?;
    if value > 0.0 {
        Ok(())
    } else {
        Err(DomainError::InvalidPrice { field, value })
    }
}

pub(crate) fn ensure_non_negative(field: &'static str, value: f64) -> Result<(), DomainError> {
    ensure_finite(field, value)?;
    if value >= 0.0 {
        Ok(())
    } else {
        Err(DomainError::Negative { field, value })
    }
}

pub(crate) fn ensure_positive_quantity(value: f64) -> Result<(), DomainError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(DomainError::InvalidQuantity { value })
    }
}
