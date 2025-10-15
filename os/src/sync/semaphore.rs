//! Semaphore

use crate::sync::UPSafeCell;
use crate::task::{block_current_and_run_next, current_task, wakeup_task, TaskControlBlock};
use alloc::vec::Vec;
use alloc::{collections::VecDeque, sync::Arc};

/// semaphore structure
pub struct Semaphore {
    /// semaphore inner
    pub inner: UPSafeCell<SemaphoreInner>,
}

pub struct SemaphoreInner {
    pub count: isize,
    pub wait_queue: VecDeque<Arc<TaskControlBlock>>,
    pub holders: Vec<(usize, isize)>,
}

impl Semaphore {
    /// Create a new semaphore
    pub fn new(res_count: usize) -> Self {
        trace!("kernel: Semaphore::new");
        Self {
            inner: unsafe {
                UPSafeCell::new(SemaphoreInner {
                    count: res_count as isize,
                    wait_queue: VecDeque::new(),
                    holders: Vec::new(),
                })
            },
        }
    }

    /// up operation of semaphore
    pub fn up(&self) {
        trace!("kernel: Semaphore::up");
        let mut inner = self.inner.exclusive_access();
        let tid = current_task().unwrap().inner_exclusive_access().res.as_ref().unwrap().tid;

        if let Some(entry) = inner.holders.iter_mut().find(|(t, _)| *t == tid) {
            if entry.1 > 0 {
                entry.1 -= 1;
            }
        }
        inner.holders.retain(|&(_, c)| c > 0);

        inner.count += 1;
        if inner.count <= 0 {
            if let Some(task) = inner.wait_queue.pop_front() {
                wakeup_task(task);
            }
        }
    }

    /// down operation of semaphore
    pub fn down(&self) {
        let mut inner = self.inner.exclusive_access();
        inner.count -= 1;

        if inner.count >= 0 {
            let tid = current_task()
                .unwrap()
                .inner_exclusive_access()
                .res
                .as_ref()
                .unwrap()
                .tid;

            if let Some(entry) = inner.holders.iter_mut().find(|(t, _)| *t == tid) {
                entry.1 += 1;
            } else {
                inner.holders.push((tid, 1));
            }
            return;
        } else {
            let task = current_task().unwrap();
            if !inner.wait_queue.iter().any(|t| Arc::ptr_eq(t, &task)) {
                inner.wait_queue.push_back(task);
            }
            drop(inner);
            block_current_and_run_next();

            let mut inner = self.inner.exclusive_access();
            let tid = current_task()
                .unwrap()
                .inner_exclusive_access()
                .res
                .as_ref()
                .unwrap()
                .tid;
            if let Some(entry) = inner.holders.iter_mut().find(|(t, _)| *t == tid) {
                entry.1 += 1;
            } else {
                inner.holders.push((tid, 1));
            }
            return;
        }
    }

    /// get_state_for_deadlock_detection
    pub fn get_state_for_deadlock_detection(&self) -> (isize, Vec<(usize, isize)>, Vec<usize>) {
        let inner = self.inner.exclusive_access();
        let available_count = if inner.count > 0 { inner.count } else { 0 };
        let holders = inner.holders.clone();
        let waiters = inner
            .wait_queue
            .iter()
            .map(|task| {
                task.inner_exclusive_access()
                    .res
                    .as_ref()
                    .unwrap()
                    .tid
            })
            .collect();
        (available_count, holders, waiters)
    }
}
