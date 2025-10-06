use spin::Mutex;

type TaskFn = fn();

static TASKS: Mutex<[Option<TaskFn>; 8]> = Mutex::new([None; 8]);
static NEXT_INDEX: Mutex<usize> = Mutex::new(0);

pub fn register(task: TaskFn) -> bool {
    let mut slots = TASKS.lock();
    for slot in slots.iter_mut() {
        if slot.is_none() {
            *slot = Some(task);
            return true;
        }
    }
    false
}

pub fn run_once() {
    let mut idx = NEXT_INDEX.lock();
    let mut slots = TASKS.lock();
    let len = slots.len();
    for _ in 0..len {
        let i = *idx % len;
        *idx = (*idx + 1) % len;
        if let Some(f) = slots[i] {
            drop(slots);
            drop(idx);
            f();
            return;
        }
    }
}

pub fn runqueue_len() -> usize {
    let slots = TASKS.lock();
    slots.iter().filter(|t| t.is_some()).count()
}

