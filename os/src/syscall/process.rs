//! Process management syscalls
use alloc::slice;

use crate::config::{MAX_SYSCALL_NUM, MEMORY_END, PAGE_SIZE};
use crate::mm::{translated_byte_buffer, MapPermission, PTEFlags, PageTable, PhysAddr, VPNRange, VirtAddr};
use crate::task::{change_program_brk, create_new_map_area, current_user_token, exit_current_and_run_next, get_current_task_page_table, suspend_current_and_run_next, unmap_consecutive_area, TASK_MANAGER};
use crate::timer::get_time_us;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
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
        buf.copy_from_slice(&ts_bytes[offset..offset+len]);
        offset += len;
    }
    0
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    let token = current_user_token();
    match trace_request {
        0 => {
            match translated_ref_u8(token, id as *const u8) {
                Some(byte_ref) => *byte_ref as isize,
                None => -1,
            }
        },
        1 => {
            match translated_refmut_u8(token, id as *mut u8) {
                Some(byte_mut) => {
                    *byte_mut = data as u8;
                    0
                }
                None => -1,
            }
        },
        2 => {
            if id >= MAX_SYSCALL_NUM {
                return  -1;
            }
                TASK_MANAGER.trace_syscalls(id)
        },
        _ => -1,
    }
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    if start % PAGE_SIZE != 0   ||
       prot & !0x7 != 0         ||    
       prot & 0x7 == 0          ||
       len == 0                 ||
       start >= MEMORY_END{   
           return -1;
    }

    let end = match start.checked_add(len) {
        Some(v) => v,
        None => return -1,
    };

    if end > MEMORY_END { return -1; }

    let mut perm = MapPermission::empty();
    if (prot & 1) != 0 {
        perm |= MapPermission::R;
    }
    if (prot & 2) != 0 {
        perm |= MapPermission::W;
    }
    if (prot & 4) != 0 {
        perm |= MapPermission::X;
    }
    perm |= MapPermission::U;

    let start_vpn = VirtAddr::from(start).floor();
    let end_vpn = VirtAddr::from(end).ceil();
    let vpns = VPNRange::new(start_vpn, end_vpn);
    for vpn in vpns {
        if let Some(pte) = get_current_task_page_table(vpn) {
            if pte.is_valid() {return -1;}
        }
    }

    create_new_map_area(
        start_vpn.into(), 
        end_vpn.into(), 
        perm,
    );

    flush_tlb();
    0
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    if start % PAGE_SIZE != 0 || len == 0 || start >= MEMORY_END{
        return -1;
    }

    let mut mlen = len;
    if start > MEMORY_END - len {
        mlen = MEMORY_END - start;
    }
    let res = unmap_consecutive_area(start, mlen);
    if res == 0 {
        flush_tlb();
    }
    res
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// Flushes the entire TLB (non-global mappings).
#[inline(always)]
pub fn flush_tlb() {
    unsafe {
        core::arch::asm!("sfence.vma");
    }
}

fn translated_ref_u8(token: usize, ptr: *const u8) -> Option<&'static u8> {
    let page_table = PageTable::from_token(token);
    let va = VirtAddr::from(ptr as usize);
    let vpn = va.floor();
    let pte = page_table.translate(vpn)?;

    if !(pte.is_valid() && pte.flags().contains(PTEFlags::U) && pte.readable()) {
        return None;
    }
    let pa: usize = PhysAddr::from(pte.ppn()).0 + va.page_offset();

    Some(unsafe { &*(pa as *const u8) })
}

fn translated_refmut_u8(token: usize, ptr: *mut u8) -> Option<&'static mut u8> {
    let page_table = PageTable::from_token(token);
    let va = VirtAddr::from(ptr as usize);
    let vpn = va.floor();
    let pte = page_table.translate(vpn)?;
    
    if !(pte.is_valid() && pte.flags().contains(PTEFlags::U) && pte.writable()) {
        return None;
    }
    let pa: usize = PhysAddr::from(pte.ppn()).0 + va.page_offset();

    Some(unsafe { &mut *(pa as *mut u8) })
}
