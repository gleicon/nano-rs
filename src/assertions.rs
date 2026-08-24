//! TigerStyle assertion framework
//!
//! Comprehensive assertion macros per TigerStyle principle:
//! - Minimum 2 assertions per function
//! - Assert positive AND negative space
//! - Pair assertions across code paths
//! - Safety-critical paths have 3+ assertions
//!
//! ## Assertion Categories
//!
//! - **Precondition**: Entry invariant (what must be true on function entry)
//! - **Postcondition**: Exit invariant (what must be true on function exit)
//! - **Positive space**: What MUST be true
//! - **Negative space**: What MUST NOT be true
//! - **State transition**: Valid state changes
//! - **Invariant**: Must hold throughout execution
//!
//! ## Usage Examples
//!
//! ```rust
//! fn validate(value: u32) {
//!     nano::assert_precondition!(value > 0, "value must be positive");
//!     nano::assert_negative!(value == 0, "value must not be zero");
//!     nano::assert_positive!(value < 1000, "value must be below limit");
//! }
//! ```

/// Positive space assertion: what MUST be true
///
/// Use this to assert that a condition that should be true IS true.
/// Panics with detailed message including file:line if condition fails.
#[macro_export]
macro_rules! assert_positive {
    ($condition:expr, $message:literal) => {
        assert!(
            $condition,
            "POSITIVE: {} at {}:{}",
            $message,
            file!(),
            line!()
        )
    };
    ($condition:expr, $($arg:tt)+) => {
        assert!(
            $condition,
            "POSITIVE: {} at {}:{}",
            format_args!($($arg)+),
            file!(),
            line!()
        )
    };
}

/// Negative space assertion: what MUST NOT be true
///
/// Use this to assert that a condition that should NOT be true IS NOT true.
/// Panics with detailed message including file:line if condition is true.
#[macro_export]
macro_rules! assert_negative {
    ($condition:expr, $message:literal) => {
        assert!(
            !$condition,
            "NEGATIVE: {} at {}:{}",
            $message,
            file!(),
            line!()
        )
    };
    ($condition:expr, $($arg:tt)+) => {
        assert!(
            !$condition,
            "NEGATIVE: {} at {}:{}",
            format_args!($($arg)+),
            file!(),
            line!()
        )
    };
}

/// Precondition assertion: entry invariant
///
/// Assert that conditions required for function entry are met.
/// These should be checked at the very beginning of the function.
#[macro_export]
macro_rules! assert_precondition {
    ($condition:expr, $message:literal) => {
        assert!(
            $condition,
            "PRECONDITION: {} at {}:{}",
            $message,
            file!(),
            line!()
        )
    };
    ($condition:expr, $($arg:tt)+) => {
        assert!(
            $condition,
            "PRECONDITION: {} at {}:{}",
            format_args!($($arg)+),
            file!(),
            line!()
        )
    };
}

/// Postcondition assertion: exit invariant
///
/// Assert that conditions required before function exit are met.
/// These should be checked before returning from the function.
#[macro_export]
macro_rules! assert_postcondition {
    ($condition:expr, $message:literal) => {
        assert!(
            $condition,
            "POSTCONDITION: {} at {}:{}",
            $message,
            file!(),
            line!()
        )
    };
    ($condition:expr, $($arg:tt)+) => {
        assert!(
            $condition,
            "POSTCONDITION: {} at {}:{}",
            format_args!($($arg)+),
            file!(),
            line!()
        )
    };
}

/// State transition assertion: valid state change
///
/// Assert that a state transition is in the set of valid transitions.
/// This helps catch invalid state machine transitions.
#[macro_export]
macro_rules! assert_state_transition {
    ($from:expr, $to:expr, $($valid:expr),+ $(,)?) => {
        {
            let from_state = $from;
            let to_state = $to;
            let valid_transitions: &[(_, _)] = &[$($valid),+];
            assert!(
                valid_transitions.contains(&(from_state, to_state)),
                "INVALID STATE TRANSITION: {:?} -> {:?} at {}:{}, valid transitions: {:?}",
                from_state, to_state, file!(), line!(), valid_transitions
            );
        }
    };
}

/// Invariant assertion: must hold throughout execution
///
/// Use this for conditions that must remain true throughout a code block.
/// Typically used in loops or long-running operations.
#[macro_export]
macro_rules! assert_invariant {
    ($condition:expr, $message:literal) => {
        assert!(
            $condition,
            "INVARIANT: {} at {}:{}",
            $message,
            file!(),
            line!()
        )
    };
    ($condition:expr, $($arg:tt)+) => {
        assert!(
            $condition,
            "INVARIANT: {} at {}:{}",
            format_args!($($arg)+),
            file!(),
            line!()
        )
    };
}

/// Debug-only TigerStyle assertion
///
/// Only active in debug builds. Use for expensive checks that shouldn't
/// run in production but are valuable during development and testing.
#[macro_export]
macro_rules! debug_assert_tiger {
    ($condition:expr, $message:literal) => {
        debug_assert!(
            $condition,
            "DEBUG: {} at {}:{}",
            $message,
            file!(),
            line!()
        )
    };
    ($condition:expr, $($arg:tt)+) => {
        debug_assert!(
            $condition,
            "DEBUG: {} at {}:{}",
            format_args!($($arg)+),
            file!(),
            line!()
        )
    };
}

/// Range assertion: value within bounds
///
/// Assert that a value is within an inclusive range [min, max].
/// Panics with detailed message if value is out of range.
#[macro_export]
macro_rules! assert_range {
    ($value:expr, $min:expr, $max:expr) => {{
        let val = $value;
        let min_val = $min;
        let max_val = $max;
        assert!(
            val >= min_val && val <= max_val,
            "RANGE: value {} not in range [{}..{}] at {}:{}",
            val,
            min_val,
            max_val,
            file!(),
            line!()
        );
    }};
}

/// Non-null assertion for pointers
///
/// Assert that a pointer is not null before dereferencing.
/// Panics with file:line information if pointer is null.
#[macro_export]
macro_rules! assert_non_null {
    ($ptr:expr) => {
        assert!(
            !$ptr.is_null(),
            "NON_NULL: pointer is null at {}:{}",
            file!(),
            line!()
        )
    };
}

/// Static allocation assertion (design-time enforcement)
///
/// Assert that memory allocation is only happening during initialization phase,
/// not during request handling. Used to enforce TigerStyle static allocation.
///
/// # Design Rationale
/// This macro is intentionally a no-op at runtime. The assertion is enforced
/// through code review and static analysis - calling this macro documents the
/// intent that a block should not allocate during request handling. It serves
/// as a visual marker for reviewers and linters.
#[macro_export]
macro_rules! assert_static_allocation_phase {
    () => {
        // Design-time assertion: no runtime check. The presence of this macro
        // call signals to reviewers that the enclosing block must not allocate
        // during request handling. Verified through code review.
    };
}

/// Thread affinity assertion
///
/// Assert that an isolate is being accessed from the thread that created it.
/// Critical for V8 safety as isolates are !Send + !Sync.
#[macro_export]
macro_rules! assert_thread_affinity {
    ($isolate_thread_id:expr) => {
        {
            let current_thread = std::thread::current().id();
            let expected_thread = $isolate_thread_id;
            assert!(
                current_thread == expected_thread,
                "THREAD AFFINITY VIOLATION: isolate accessed from wrong thread. Expected {:?}, got {:?} at {}:{}",
                expected_thread, current_thread, file!(), line!()
            );
        }
    };
}

/// Resource limit assertion
///
/// Assert that a resource usage is within the specified limit.
/// Used to enforce TigerStyle explicit resource limits.
#[macro_export]
macro_rules! assert_resource_limit {
    ($usage:expr, $limit:expr, $resource_name:expr) => {{
        let usage_val = $usage;
        let limit_val = $limit;
        assert!(
            usage_val <= limit_val,
            "RESOURCE LIMIT EXCEEDED: {} usage {} exceeds limit {} at {}:{}",
            $resource_name,
            usage_val,
            limit_val,
            file!(),
            line!()
        );
    }};
}

/// Loop iteration limit assertion
///
/// Used in loops to assert that iteration count doesn't exceed maximum.
/// Prevents infinite loops per TigerStyle no-recursion principle.
#[macro_export]
macro_rules! assert_iteration_limit {
    ($counter:expr, $max:expr) => {{
        let counter_val = $counter;
        let max_val = $max;
        assert!(
            counter_val < max_val,
            "ITERATION LIMIT EXCEEDED: {} iterations (max {}) at {}:{}",
            counter_val,
            max_val,
            file!(),
            line!()
        );
    }};
}

/// Re-export all assertion macros for convenient use
pub mod prelude {
    pub use crate::{
        assert_invariant, assert_iteration_limit, assert_negative, assert_non_null,
        assert_positive, assert_postcondition, assert_precondition, assert_range,
        assert_resource_limit, assert_state_transition, assert_static_allocation_phase,
        assert_thread_affinity, debug_assert_tiger,
    };
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_positive_assertion_passes() {
        assert_positive!(true, "should pass");
    }

    #[test]
    #[should_panic(expected = "POSITIVE:")]
    fn test_positive_assertion_fails() {
        assert_positive!(false, "should fail");
    }

    #[test]
    fn test_negative_assertion_passes() {
        assert_negative!(false, "should pass");
    }

    #[test]
    #[should_panic(expected = "NEGATIVE:")]
    fn test_negative_assertion_fails() {
        assert_negative!(true, "should fail");
    }

    #[test]
    fn test_precondition_assertion_passes() {
        assert_precondition!(true, "valid precondition");
    }

    #[test]
    #[should_panic(expected = "PRECONDITION:")]
    fn test_precondition_assertion_fails() {
        assert_precondition!(false, "invalid precondition");
    }

    #[test]
    fn test_postcondition_assertion_passes() {
        assert_postcondition!(true, "valid postcondition");
    }

    #[test]
    #[should_panic(expected = "POSTCONDITION:")]
    fn test_postcondition_assertion_fails() {
        assert_postcondition!(false, "invalid postcondition");
    }

    #[test]
    fn test_range_assertion_passes() {
        assert_range!(50, 0, 100);
        assert_range!(0, 0, 100); // boundary
        assert_range!(100, 0, 100); // boundary
    }

    #[test]
    #[should_panic(expected = "RANGE:")]
    fn test_range_assertion_fails_low() {
        assert_range!(-1i32, 0, 100);
    }

    #[test]
    #[should_panic(expected = "RANGE:")]
    fn test_range_assertion_fails_high() {
        assert_range!(101, 0, 100);
    }

    #[test]
    fn test_non_null_assertion_passes() {
        let x = 42;
        let ptr: *const i32 = &x;
        assert_non_null!(ptr);
    }

    #[test]
    #[should_panic(expected = "NON_NULL:")]
    fn test_non_null_assertion_fails() {
        let ptr: *const i32 = std::ptr::null();
        assert_non_null!(ptr);
    }

    #[test]
    fn test_invariant_assertion_passes() {
        assert_invariant!(true, "invariant holds");
    }

    #[test]
    #[should_panic(expected = "INVARIANT:")]
    fn test_invariant_assertion_fails() {
        assert_invariant!(false, "invariant violated");
    }

    #[test]
    fn test_resource_limit_assertion_passes() {
        assert_resource_limit!(50, 100, "memory");
    }

    #[test]
    #[should_panic(expected = "RESOURCE LIMIT EXCEEDED:")]
    fn test_resource_limit_assertion_fails() {
        assert_resource_limit!(150, 100, "memory");
    }

    #[test]
    fn test_iteration_limit_assertion_passes() {
        assert_iteration_limit!(5, 10);
    }

    #[test]
    #[should_panic(expected = "ITERATION LIMIT EXCEEDED:")]
    fn test_iteration_limit_assertion_fails() {
        assert_iteration_limit!(10, 10);
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    enum TestState {
        Idle,
        Processing,
        Complete,
        Error,
    }

    #[test]
    fn test_state_transition_valid() {
        let from = TestState::Idle;
        let to = TestState::Processing;
        assert_state_transition!(
            from,
            to,
            (TestState::Idle, TestState::Processing),
            (TestState::Processing, TestState::Complete),
            (TestState::Processing, TestState::Error),
        );
    }

    #[test]
    #[should_panic(expected = "INVALID STATE TRANSITION:")]
    fn test_state_transition_invalid() {
        let from = TestState::Complete;
        let to = TestState::Processing;
        assert_state_transition!(
            from,
            to,
            (TestState::Idle, TestState::Processing),
            (TestState::Processing, TestState::Complete),
        );
    }
}
