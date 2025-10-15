use crate::sync::{Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore};
use crate::task::{block_current_and_run_next, current_process, current_task};
use crate::timer::{add_timer, get_time_ms};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
/// sleep syscall
pub fn sys_sleep(ms: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_sleep",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let expire_ms = get_time_ms() + ms;
    let task = current_task().unwrap();
    add_timer(expire_ms, task);
    block_current_and_run_next();
    0
}
/// mutex create syscall
pub fn sys_mutex_create(blocking: bool) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mutex: Option<Arc<dyn Mutex>> = if !blocking {
        Some(Arc::new(MutexSpin::new()))
    } else {
        Some(Arc::new(MutexBlocking::new()))
    };
    let mut process_inner = process.inner_exclusive_access();
    if let Some(id) = process_inner
        .mutex_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.mutex_list[id] = mutex;
        id as isize
    } else {
        process_inner.mutex_list.push(mutex);
        process_inner.mutex_list.len() as isize - 1
    }
}
/// mutex lock syscall
pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    let process = current_process();
    let mutex = Arc::clone(
        process.inner_exclusive_access().mutex_list[mutex_id]
            .as_ref()
            .unwrap(),
    );

    let process_inner = process.inner_exclusive_access();
    if process_inner.deadlock_detect_enable {
        let task = current_task().unwrap();
        let tid = task.inner_exclusive_access().res.as_ref().unwrap().tid;
        let tasks_num = process_inner.tasks.len();
        let resources_num = process_inner.mutex_list.len();

        let mut available = vec![1; resources_num];
        let mut allocation: Vec<Vec<isize>> = vec![vec![0; resources_num]; tasks_num];
        let mut need: Vec<Vec<isize>> = vec![vec![0; resources_num]; tasks_num];

        for i in 0..resources_num {
            if let Some(m) = &process_inner.mutex_list[i] {
                let (owner_tid, waiter_tids) = m.get_owner_and_waiters();
                if let Some(owner) = owner_tid {
                    available[i] = 0;
                    allocation[owner][i] = 1;
                }
                for waiter in waiter_tids {
                    need[waiter][i] = 1;
                }
            }
        }

        if allocation[tid][mutex_id] > 0 {
            return -0xDEAD;
        }

        need[tid][mutex_id] = 1;

        if available[mutex_id] == 0 {
            if !is_safe(tasks_num, resources_num, available, &allocation, &need) {
                return -0xDEAD;
            }
        }
    }

    drop(process_inner);

    mutex.lock();
    0
}
/// mutex unlock syscall
pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_unlock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    drop(process);
    mutex.unlock();
    0
}
/// semaphore create syscall
pub fn sys_semaphore_create(res_count: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .semaphore_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
        id
    } else {
        process_inner
            .semaphore_list
            .push(Some(Arc::new(Semaphore::new(res_count))));
        process_inner.semaphore_list.len() - 1
    };
    id as isize
}
/// semaphore up syscall
pub fn sys_semaphore_up(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_up",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    drop(process_inner);
    sem.up();
    0
}
/// semaphore down syscall
pub fn sys_semaphore_down(sem_id: usize) -> isize {
    let process = current_process();
    let process_inner = process.inner_exclusive_access(); // 使用 mut

    if process_inner.deadlock_detect_enable {
        let task = current_task().unwrap();
        let tid = task.inner_exclusive_access().res.as_ref().unwrap().tid;
        let tasks_num = process_inner.tasks.len();
        let resources_num = process_inner.semaphore_list.len();

        let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
        let sem_inner = sem.inner.exclusive_access();
        let will_block = sem_inner.count <= 0;
        drop(sem_inner);

        if will_block {
            let mut available = vec![0isize; resources_num];
            let mut allocation: Vec<Vec<isize>> = vec![vec![0; resources_num]; tasks_num];
            let mut need: Vec<Vec<isize>> = vec![vec![0; resources_num]; tasks_num];
            
            for i in 0..resources_num {
                if let Some(s) = &process_inner.semaphore_list[i] {
                    let (count, holders, waiters) = s.get_state_for_deadlock_detection();
                    available[i] = count;
                    for &(holder_tid, hold_count) in &holders {
                        if holder_tid < tasks_num {
                            allocation[holder_tid][i] = hold_count;
                        }
                    }
                    for &waiter_tid in &waiters {
                         if waiter_tid < tasks_num {
                            need[waiter_tid][i] = 1;
                        }
                    }
                }
            }

            need[tid][sem_id] = 1;

            if !is_safe(tasks_num, resources_num, available, &allocation, &need) {
                return -0xDEAD;
            }
        }
    }

    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    drop(process_inner);

    sem.down();
    0
}
/// condvar create syscall
pub fn sys_condvar_create() -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .condvar_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.condvar_list[id] = Some(Arc::new(Condvar::new()));
        id
    } else {
        process_inner
            .condvar_list
            .push(Some(Arc::new(Condvar::new())));
        process_inner.condvar_list.len() - 1
    };
    id as isize
}
/// condvar signal syscall
pub fn sys_condvar_signal(condvar_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_signal",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    drop(process_inner);
    condvar.signal();
    0
}
/// condvar wait syscall
pub fn sys_condvar_wait(condvar_id: usize, mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_wait",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    condvar.wait(mutex);
    0
}
/// enable deadlock detection syscall
///
/// YOUR JOB: Implement deadlock detection, but might not all in this syscall
pub fn sys_enable_deadlock_detect(enabled: usize) -> isize {
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    if enabled == 1 {
        process_inner.deadlock_detect_enable = true;
    } else if enabled == 0 {
        process_inner.deadlock_detect_enable = false;
    } else {
        return -1;
    }
    0
}

fn is_safe(
    tasks_num: usize,
    resources_num: usize,
    avaliable: Vec<isize>,
    allocation: &Vec<Vec<isize>>,
    need: &Vec<Vec<isize>>,
) -> bool {
    let mut work = avaliable.clone();
    let mut finish = vec![false; tasks_num];
    loop {
        let mut found = false;
        for i in 0..tasks_num {
            if !finish[i] {
                let mut possible = true;
                for j in 0..resources_num {
                    if need[i][j] > work[j] {
                        possible = false;
                        break;
                    }
                }
                if possible {
                    for j in 0..resources_num {
                        work[j] += allocation[i][j];
                    }
                    finish[i] = true;
                    found = true;
                }
            }
        }
        if !found {
            break;
        }
    }
    for i in 0..tasks_num {
        if !finish[i] {
            return false;
        }
    }
    true
}
