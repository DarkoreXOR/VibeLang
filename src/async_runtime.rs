use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, VecDeque};

use crate::error::Span;
use crate::value::Value;

pub type TaskId = u64;

#[derive(Debug, Clone)]
struct TimerEntry {
    deadline_ms: u64,
    task_id: TaskId,
}

impl PartialEq for TimerEntry {
    fn eq(&self, other: &Self) -> bool {
        self.deadline_ms == other.deadline_ms && self.task_id == other.task_id
    }
}
impl Eq for TimerEntry {}

impl PartialOrd for TimerEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TimerEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // BinaryHeap is max-heap; invert for min-heap behavior via Reverse.
        self.deadline_ms
            .cmp(&other.deadline_ms)
            .then_with(|| self.task_id.cmp(&other.task_id))
    }
}

#[derive(Debug)]
pub struct RuntimeError {
    pub message: String,
    pub span: Option<Span>,
}

impl RuntimeError {
    pub fn new(message: impl Into<String>, span: Option<Span>) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

#[derive(Debug)]
pub enum TaskState<TCtx> {
    Ready(TCtx),
    WaitingOn { ctx: TCtx, awaited: TaskId },
    SleepingUntil { ctx: TCtx, deadline_ms: u64 },
    Completed(Value),
    Failed(RuntimeError),
}

#[derive(Debug)]
pub struct AsyncRuntime<TCtx> {
    next_id: TaskId,
    tasks: HashMap<TaskId, TaskState<TCtx>>,
    ready: VecDeque<TaskId>,
    timers: BinaryHeap<Reverse<TimerEntry>>,
    waiters: HashMap<TaskId, Vec<TaskId>>,
}

impl<TCtx> AsyncRuntime<TCtx> {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            tasks: HashMap::new(),
            ready: VecDeque::new(),
            timers: BinaryHeap::new(),
            waiters: HashMap::new(),
        }
    }

    pub fn alloc_id(&mut self) -> TaskId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn spawn_ready(&mut self, ctx: TCtx) -> TaskId {
        let id = self.alloc_id();
        self.tasks.insert(id, TaskState::Ready(ctx));
        self.ready.push_back(id);
        id
    }

    pub fn spawn_sleeping(&mut self, ctx: TCtx, deadline_ms: u64) -> TaskId {
        let id = self.alloc_id();
        self.set_sleeping_until(id, ctx, deadline_ms);
        id
    }

    pub fn is_completed(&self, id: TaskId) -> bool {
        matches!(self.tasks.get(&id), Some(TaskState::Completed(_)))
    }

    pub fn completed_value(&self, id: TaskId) -> Option<&Value> {
        match self.tasks.get(&id) {
            Some(TaskState::Completed(v)) => Some(v),
            _ => None,
        }
    }

    pub fn fail(&mut self, id: TaskId, err: RuntimeError) {
        self.tasks.insert(id, TaskState::Failed(err));
        // Wake any waiters so they can observe/propagate failure.
        if let Some(ws) = self.waiters.remove(&id) {
            for w in ws {
                // Only wake tasks that are still waiting on this exact awaited task.
                let awaited_matches = matches!(
                    self.tasks.get(&w),
                    Some(TaskState::WaitingOn { awaited, .. }) if *awaited == id
                );
                if awaited_matches {
                    if let Some(TaskState::WaitingOn { ctx, awaited }) =
                        self.tasks.remove(&w)
                    {
                        debug_assert_eq!(awaited, id);
                        self.tasks.insert(w, TaskState::Ready(ctx));
                        self.ready.push_back(w);
                    }
                }
            }
        }
    }

    pub fn complete(&mut self, id: TaskId, value: Value) {
        self.tasks.insert(id, TaskState::Completed(value));
        if let Some(ws) = self.waiters.remove(&id) {
            for w in ws {
                // Only wake tasks that are still waiting on this exact awaited task.
                let awaited_matches = matches!(
                    self.tasks.get(&w),
                    Some(TaskState::WaitingOn { awaited, .. }) if *awaited == id
                );
                if awaited_matches {
                    if let Some(TaskState::WaitingOn { ctx, awaited }) =
                        self.tasks.remove(&w)
                    {
                        debug_assert_eq!(awaited, id);
                        self.tasks.insert(w, TaskState::Ready(ctx));
                        self.ready.push_back(w);
                    }
                }
            }
        }
    }

    pub fn pop_ready(&mut self) -> Option<TaskId> {
        self.ready.pop_front()
    }

    pub fn take_ready_ctx(&mut self, id: TaskId) -> Option<TCtx> {
        match self.tasks.remove(&id) {
            Some(TaskState::Ready(ctx)) => Some(ctx),
            other => {
                if let Some(o) = other {
                    self.tasks.insert(id, o);
                }
                None
            }
        }
    }

    pub fn set_ready(&mut self, id: TaskId, ctx: TCtx) {
        self.tasks.insert(id, TaskState::Ready(ctx));
        self.ready.push_back(id);
    }

    pub fn set_waiting_on(&mut self, id: TaskId, ctx: TCtx, awaited: TaskId) {
        self.tasks
            .insert(id, TaskState::WaitingOn { ctx, awaited });
        self.waiters.entry(awaited).or_default().push(id);
    }

    pub fn set_sleeping_until(&mut self, id: TaskId, ctx: TCtx, deadline_ms: u64) {
        self.tasks
            .insert(id, TaskState::SleepingUntil { ctx, deadline_ms });
        self.timers.push(Reverse(TimerEntry { deadline_ms, task_id: id }));
    }

    pub fn wake_expired_timers(&mut self, now_ms: u64) {
        while let Some(Reverse(top)) = self.timers.peek().cloned() {
            if top.deadline_ms > now_ms {
                break;
            }
            let _ = self.timers.pop();
            let tid = top.task_id;
            // If still sleeping, wake it.
            let Some(state) = self.tasks.remove(&tid) else { continue };
            match state {
                TaskState::SleepingUntil { ctx, deadline_ms } => {
                    if deadline_ms <= now_ms {
                        self.tasks.insert(tid, TaskState::Ready(ctx));
                        self.ready.push_back(tid);
                    } else {
                        // Not yet (race with updated sleep). Put back.
                        self.tasks.insert(tid, TaskState::SleepingUntil { ctx, deadline_ms });
                        self.timers.push(Reverse(TimerEntry { deadline_ms, task_id: tid }));
                    }
                }
                other => {
                    // Timer entry is stale.
                    self.tasks.insert(tid, other);
                }
            }
        }
    }

    pub fn next_timer_deadline_ms(&self) -> Option<u64> {
        self.timers.peek().map(|Reverse(e)| e.deadline_ms)
    }

    pub fn take_waiting_ctx_if_awakened(&mut self, id: TaskId) -> Option<(TCtx, TaskId)> {
        match self.tasks.remove(&id) {
            Some(TaskState::WaitingOn { ctx, awaited }) => Some((ctx, awaited)),
            other => {
                if let Some(o) = other {
                    self.tasks.insert(id, o);
                }
                None
            }
        }
    }
}

