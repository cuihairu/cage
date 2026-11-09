//! L0 parse/syntax validation pass.
//! Split out of the validation module (LAY-003).

use super::ValidationContext;

pub(crate) fn validate_parse(_ctx: &mut ValidationContext) {
    // Source adapters should have caught parse errors
    // This level is a no-op in core, but exists for completeness
}
