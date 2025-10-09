//! Process management syscalls
use alloc::{slice, sync::Arc, vec::Vec};

use crate::{
    config::{MEMORY_END, PAGE_SIZE, TRAP_CONTEXT_BASE}, loader::get_app_data_by_name, mm::{translated_byte_buffer, translated_refmut, translated_str, MapPermission, MemorySet, VPNRange, VirtAddr, KERNEL_SPACE}, sync::UPSafeCell, task::{
        add_task, create_new_map_area, current_task, current_user_token, exit_current_and_run_next, get_current_task_page_table, kstack_alloc, pid_alloc, suspend_current_and_run_next, unmap_consecutive_area, TaskContext, TaskControlBlock, TaskControlBlockInner, TaskStatus, BIG_STRIDE
    }, timer::get_time_us, trap::{TrapContext, trap_handler}
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel:pid[{}] sys_yield", current_task().unwrap().pid.0);
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(data) = get_app_data_by_name(path.as_str()) {
        let task = current_task().unwrap();
        task.exec(data);
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    trace!("kernel::pid[{}] sys_waitpid [{}]", current_task().unwrap().pid.0, pid);
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    let us = get_time_us();

    let time_val = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };

    let token = current_user_token();

    let ts_ptr = ts as *const u8;
    let ts_bytes = unsafe {
        slice::from_raw_parts(
            &time_val as *const TimeVal as *const u8, 
            core::mem::size_of::<TimeVal>(),
        )
    };

    let mut buffers = translated_byte_buffer(token, ts_ptr, ts_bytes.len());
    let mut offset = 0;
    for buf in buffers.iter_mut() {
        let len = buf.len();
        buf.copy_from_slice(&ts_bytes[offset..offset + len]);
        offset += len;
    }
    0
}

/// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, port: usize) -> isize {
    if start % PAGE_SIZE != 0   ||
       port & !0x7 != 0         ||
       port & 0x7 == 0          ||
       len == 0                 ||
       start >= MEMORY_END { return -1; }

    let end = match start.checked_add(len) {
        Some(v) => v,
        None => return -1,
    };

    if end > MEMORY_END {return -1;}

    let mut perm = MapPermission::empty();
    if port & 1 != 0 { perm |= MapPermission::R; }
    if port & 2 != 0 { perm |= MapPermission::W; }
    if port & 4 != 0 { perm |= MapPermission::X; }
    perm |= MapPermission::U;

    let start_vpn = VirtAddr::from(start).floor();
    let end_vpn = VirtAddr::from(end).ceil();
    let vpns = VPNRange::new(start_vpn, end_vpn);
    for vpn in vpns {
        if let Some(pte) = get_current_task_page_table(vpn) {
            if pte.is_valid() { return -1; }
        }
    }

    create_new_map_area(
        start_vpn.into(), 
        end_vpn.into(),
        perm
    );

    flush_tlb();
    0
}

/// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    if start % PAGE_SIZE != 0   ||
       len == 0                 ||
       start >= MEMORY_END { return -1; }

       let mut mlen = len;
       if start > MEMORY_END - len { mlen = MEMORY_END - start; }
       let res = unmap_consecutive_area(start, mlen);
       if res == 0 {
        flush_tlb();
       }
       res
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
pub fn sys_spawn(path: *const u8) -> isize {
    let task = current_task().unwrap();
    let mut parent_inner = task.inner_exclusive_access();
    let token = parent_inner.memory_set.token();
    let path = translated_str(token, path);

    if let Some(elf_data) = get_app_data_by_name(path.as_str()) {
        let (memory_set, user_sp, entry_point) = MemorySet::from_elf(elf_data);
        let trap_cx_ppn = memory_set
            .translate(VirtAddr::from(TRAP_CONTEXT_BASE).into())
            .unwrap().ppn();

        let pid_handle = pid_alloc();
        let kernel_stack = kstack_alloc();
        let kernel_stack_top = kernel_stack.get_top();
        let tcb = Arc::new(TaskControlBlock {
            pid: pid_handle,
            kernel_stack,
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    trap_cx_ppn,
                    base_size: parent_inner.base_size,
                    task_cx: TaskContext::goto_trap_return(kernel_stack_top),
                    task_status: TaskStatus::Ready,
                    memory_set,
                    parent: Some(Arc::downgrade(&task)),
                    children: Vec::new(),
                    exit_code: 0,
                    heap_bottom: parent_inner.heap_bottom,
                    program_brk: parent_inner.program_brk,
                    stride: 0,
                    priority: parent_inner.priority,
                    pass: BIG_STRIDE / (parent_inner.priority as usize),

                })
            }
        });

        parent_inner.children.push(tcb.clone());

        let trap_cx = tcb.inner_exclusive_access().get_trap_cx();
        *trap_cx = TrapContext::app_init_context(
            entry_point, 
            user_sp, 
            KERNEL_SPACE.exclusive_access().token(), 
            kernel_stack_top, 
            trap_handler as usize
        );
        let pid = tcb.pid.0 as isize;
        add_task(tcb);
        pid
    } else {
        -1
    }
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(prio: isize) -> isize {
    if prio < 2 {
        return -1;
    }

    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    inner.priority = prio as usize;
    inner.pass = BIG_STRIDE / (prio as usize); 
    prio
}

/// flush tlb
#[inline(always)]
pub fn flush_tlb() {
    unsafe {
        core::arch::asm!("sfence.vma");
    }
}

