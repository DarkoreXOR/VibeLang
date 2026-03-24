use crate::error::{SemanticError, Span};

#[allow(dead_code)]
pub(crate) fn report_generic_struct_pattern_mismatch(
    expected_name: &str,
    pattern_full_name: &str,
    span: Span,
    errors: &mut Vec<SemanticError>,
) {
    errors.push(SemanticError::new(
        format!(
            "struct pattern type mismatch: expected `{}`, found `{}`",
            expected_name, pattern_full_name
        ),
        span,
    ));
}

