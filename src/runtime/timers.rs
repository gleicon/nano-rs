//! setTimeout / setInterval / clearTimeout / clearInterval V8 bindings

use std::cell::{Cell, RefCell};
use std::time::Instant;

struct TimeoutEntry {
    id: u32,
    func: v8::Global<v8::Function>,
    fire_at: Instant,
}

struct IntervalEntry {
    id: u32,
    func: v8::Global<v8::Function>,
    interval_ms: u64,
    next_fire: Instant,
}

thread_local! {
    /// Live setTimeout entries for the current request.
    static PENDING_TIMEOUTS: RefCell<Vec<TimeoutEntry>> = const { RefCell::new(Vec::new()) };

    /// Monotonically increasing ID source for setTimeout handles (1–99).
    static TIMEOUT_ID_COUNTER: Cell<u32> = const { Cell::new(1) };

    /// Live setInterval entries for the current request.
    static PENDING_INTERVALS: RefCell<Vec<IntervalEntry>> = const { RefCell::new(Vec::new()) };

    /// IDs cleared via clearInterval() while fire_pending_intervals() is
    /// dispatching (i.e., the interval's own callback called clearInterval).
    static INTERVALS_CLEARED_DURING_FIRE: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };

    /// Monotonically increasing ID source for interval handles (100+).
    static INTERVAL_ID_COUNTER: Cell<u32> = const { Cell::new(100) };
}

/// Fire all setInterval callbacks whose next_fire deadline has passed.
///
/// Drain-and-reinsert: due entries are removed before callbacks run so that
/// `clearInterval()` from within a callback can safely mutate the vec.
pub(crate) fn fire_pending_intervals(scope: &mut v8::PinnedRef<v8::HandleScope>) {
    let now = Instant::now();

    let due: Vec<IntervalEntry> = PENDING_INTERVALS.with(|iv| {
        let mut entries = iv.borrow_mut();
        let mut due = Vec::new();
        let mut i = 0;
        while i < entries.len() {
            if now >= entries[i].next_fire {
                due.push(entries.remove(i));
            } else {
                i += 1;
            }
        }
        due
    });

    for mut entry in due {
        let func = v8::Local::new(scope, &entry.func);
        let gobj = scope.get_current_context().global(scope);
        let _ = func.call(scope, gobj.into(), &[]);

        let cleared = INTERVALS_CLEARED_DURING_FIRE.with(|cs| cs.borrow().contains(&entry.id));
        if !cleared {
            entry.next_fire = Instant::now()
                + std::time::Duration::from_millis(entry.interval_ms);
            PENDING_INTERVALS.with(|iv| iv.borrow_mut().push(entry));
        }
    }

    INTERVALS_CLEARED_DURING_FIRE.with(|cs| cs.borrow_mut().clear());
}

/// Clear all pending intervals. Call at the start of each request.
pub(crate) fn clear_pending_intervals() {
    PENDING_INTERVALS.with(|iv| iv.borrow_mut().clear());
    INTERVALS_CLEARED_DURING_FIRE.with(|cs| cs.borrow_mut().clear());
    INTERVAL_ID_COUNTER.with(|c| c.set(100));
}

/// Fire all setTimeout callbacks whose fire_at deadline has passed.
///
/// If `func.call` returns `None` (CPU guard terminated V8 mid-call), the entry
/// is re-queued — it fires on the next pump iteration after `cancel_terminate_execution`.
pub(crate) fn fire_pending_timeouts(scope: &mut v8::PinnedRef<v8::HandleScope>) {
    let now = Instant::now();

    let due: Vec<TimeoutEntry> = PENDING_TIMEOUTS.with(|tv| {
        let mut entries = tv.borrow_mut();
        let mut due = Vec::new();
        let mut i = 0;
        while i < entries.len() {
            if now >= entries[i].fire_at {
                due.push(entries.remove(i));
            } else {
                i += 1;
            }
        }
        due
    });

    let mut failed: Vec<TimeoutEntry> = Vec::new();
    for entry in due {
        let func = v8::Local::new(scope, &entry.func);
        let gobj = scope.get_current_context().global(scope);
        if func.call(scope, gobj.into(), &[]).is_none() {
            failed.push(entry);
        }
    }

    PENDING_TIMEOUTS.with(|tv| tv.borrow_mut().extend(failed));
}

/// Clear all pending timeouts. Call at the start of each request.
pub(crate) fn clear_pending_timeouts() {
    PENDING_TIMEOUTS.with(|tv| tv.borrow_mut().clear());
    TIMEOUT_ID_COUNTER.with(|c| c.set(1));
}

/// Bind setTimeout, setInterval, clearTimeout, clearInterval to the V8 global.
pub(crate) fn bind_timers(
    scope: &mut v8::PinnedRef<v8::HandleScope<()>>,
    context: v8::Local<v8::Context>,
) {
    let global = context.global(scope);
    let mut ctx_scope = v8::ContextScope::new(scope, context);

    if let Some(f) = v8::Function::new(&mut ctx_scope, set_timeout_callback) {
        let key = v8::String::new(&mut ctx_scope, "setTimeout").unwrap();
        global.set(&mut ctx_scope, key.into(), f.into());
    }
    if let Some(f) = v8::Function::new(&mut ctx_scope, set_interval_callback) {
        let key = v8::String::new(&mut ctx_scope, "setInterval").unwrap();
        global.set(&mut ctx_scope, key.into(), f.into());
    }
    if let Some(f) = v8::Function::new(&mut ctx_scope, clear_timeout_callback) {
        let key = v8::String::new(&mut ctx_scope, "clearTimeout").unwrap();
        global.set(&mut ctx_scope, key.into(), f.into());
    }
    if let Some(f) = v8::Function::new(&mut ctx_scope, clear_interval_callback) {
        let key = v8::String::new(&mut ctx_scope, "clearInterval").unwrap();
        global.set(&mut ctx_scope, key.into(), f.into());
    }
}

#[cfg(test)]
pub(crate) fn pending_timeout_count() -> usize {
    PENDING_TIMEOUTS.with(|tv| tv.borrow().len())
}

fn set_timeout_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    if args.length() == 0 || !args.get(0).is_function() {
        retval.set(v8::Number::new(scope, 0.0).into());
        return;
    }

    let delay_ms: u64 = if args.length() > 1 {
        if let Some(n) = args.get(1).to_number(scope) {
            n.value().max(0.0) as u64
        } else {
            0
        }
    } else {
        0
    };

    let func = args.get(0).cast::<v8::Function>();
    let func_global = v8::Global::new(scope, func);

    let id = TIMEOUT_ID_COUNTER.with(|c| {
        let id = c.get();
        c.set(if id >= 99 { 1 } else { id + 1 });
        id
    });

    PENDING_TIMEOUTS.with(|tv| {
        tv.borrow_mut().push(TimeoutEntry {
            id,
            func: func_global,
            fire_at: Instant::now() + std::time::Duration::from_millis(delay_ms),
        });
    });

    retval.set(v8::Number::new(scope, f64::from(id)).into());
}

fn set_interval_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    if args.length() == 0 || !args.get(0).is_function() {
        retval.set(v8::Number::new(scope, 0.0).into());
        return;
    }

    let interval_ms: u64 = if args.length() > 1 {
        if let Some(n) = args.get(1).to_number(scope) {
            n.value().max(0.0) as u64
        } else {
            0
        }
    } else {
        0
    };

    let func = args.get(0).cast::<v8::Function>();
    let func_global = v8::Global::new(scope, func);

    let id = INTERVAL_ID_COUNTER.with(|c| {
        let id = c.get();
        c.set(id.wrapping_add(1));
        id
    });

    PENDING_INTERVALS.with(|iv| {
        iv.borrow_mut().push(IntervalEntry {
            id,
            func: func_global,
            interval_ms,
            next_fire: Instant::now() + std::time::Duration::from_millis(interval_ms),
        });
    });

    retval.set(v8::Number::new(scope, f64::from(id)).into());
}

fn clear_timeout_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    if args.length() == 0 { return; }
    if let Some(n) = args.get(0).to_number(scope) {
        let target_id = n.value() as u32;
        PENDING_TIMEOUTS.with(|tv| {
            tv.borrow_mut().retain(|e| e.id != target_id);
        });
    }
}

fn clear_interval_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    if args.length() == 0 { return; }
    if let Some(n) = args.get(0).to_number(scope) {
        let target_id = n.value() as u32;
        PENDING_INTERVALS.with(|iv| {
            iv.borrow_mut().retain(|e| e.id != target_id);
        });
        INTERVALS_CLEARED_DURING_FIRE.with(|cs| {
            let mut v = cs.borrow_mut();
            if !v.contains(&target_id) {
                v.push(target_id);
            }
        });
    }
}
