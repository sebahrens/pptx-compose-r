use pptx_compose_core::error::{Error, ErrorCode, Result};

use crate::patch::Bounds;

pub const MAX_EMU_COORDINATE: i64 = 27_273_042_316_900;

pub fn validate_bounds(bounds: &Bounds) -> Result<()> {
    if bounds.x < 0 || bounds.y < 0 || bounds.cx <= 0 || bounds.cy <= 0 {
        return Err(Error::new(
            ErrorCode::InvalidBounds,
            "Bounds require x/y >= 0 and cx/cy > 0.",
        ));
    }

    if [bounds.x, bounds.y, bounds.cx, bounds.cy]
        .into_iter()
        .any(|value| value > MAX_EMU_COORDINATE)
    {
        return Err(Error::new(
            ErrorCode::InvalidBounds,
            format!("Bounds must be <= {MAX_EMU_COORDINATE} EMUs."),
        ));
    }

    if bounds
        .x
        .checked_add(bounds.cx)
        .is_none_or(|right| right > MAX_EMU_COORDINATE)
        || bounds
            .y
            .checked_add(bounds.cy)
            .is_none_or(|bottom| bottom > MAX_EMU_COORDINATE)
    {
        return Err(Error::new(
            ErrorCode::InvalidBounds,
            format!("Bounds x+cx and y+cy must be <= {MAX_EMU_COORDINATE} EMUs."),
        ));
    }

    Ok(())
}
