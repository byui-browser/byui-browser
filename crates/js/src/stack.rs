//! Native stack management for the recursive parser and interpreter.
//!
//! Both recurse once per nesting level, and unoptimized builds use several
//! kilobytes of native stack per level, far more than a 1 MiB main thread can
//! hold for realistic scripts. Entry points therefore run on a dedicated
//! thread with a large stack, and the interpreter measures how much of it is
//! in use so runaway recursion becomes a JavaScript error instead of a
//! process-aborting stack overflow.

use std::cell::Cell;
use std::sync::Mutex;

/// Stack reserved for the engine thread. Only touched pages are committed.
const ENGINE_STACK_BYTES: usize = 64 * 1024 * 1024;

/// Stack kept free below the interpreter's limit for host functions and for
/// frames between two depth checks.
const ENGINE_STACK_HEADROOM: usize = 8 * 1024 * 1024;

/// Conservative budget used when the engine runs on a caller's thread whose
/// stack size is unknown (only if spawning the engine thread fails).
const FALLBACK_STACK_BYTES: usize = 256 * 1024;

thread_local! {
    static STACK_BUDGET: Cell<usize> = const { Cell::new(FALLBACK_STACK_BYTES) };
}

/// Runs `task` on an engine thread with a large stack and returns its result.
///
/// Panics inside `task` are propagated to the caller. If the thread cannot be
/// spawned, `task` runs on the current thread with a small stack budget.
pub(crate) fn with_engine_stack<T: Send>(task: impl FnOnce() -> T + Send) -> T {
    let task = Mutex::new(Some(task));
    let take_task = || {
        task.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .expect("engine task runs once")
    };
    std::thread::scope(|scope| {
        let spawned = std::thread::Builder::new()
            .name("js-engine".into())
            .stack_size(ENGINE_STACK_BYTES)
            .spawn_scoped(scope, || {
                STACK_BUDGET.set(ENGINE_STACK_BYTES - ENGINE_STACK_HEADROOM);
                take_task()()
            });
        match spawned {
            Ok(handle) => handle
                .join()
                .unwrap_or_else(|payload| std::panic::resume_unwind(payload)),
            Err(_) => take_task()(),
        }
    })
}

/// Tracks native stack use from the point it was created.
pub(crate) struct StackGuard {
    base: usize,
    budget: usize,
}

impl StackGuard {
    /// Starts measuring from the current stack position.
    pub(crate) fn new() -> Self {
        Self {
            base: stack_position(),
            budget: STACK_BUDGET.get(),
        }
    }

    /// Returns whether the stack has grown past this thread's budget.
    pub(crate) fn exhausted(&self) -> bool {
        self.base.abs_diff(stack_position()) > self.budget
    }
}

/// An address on the current stack frame, used only for distance arithmetic.
#[inline(never)]
fn stack_position() -> usize {
    let marker = 0_u8;
    std::hint::black_box(&marker) as *const u8 as usize
}
