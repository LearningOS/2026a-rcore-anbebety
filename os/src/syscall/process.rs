//! Process management syscalls
//! Codex-assisted completion against chapter 4 requirements:
//! https://learningos.cn/rCore-Tutorial-Guide/chapter4/7exercise.html
use crate::config::PAGE_SIZE;

use crate::mm::{check_user_buffer, translated_byte_buffer, MapPermission, PTEFlags, VirtAddr};

use crate::task::{
    change_program_brk, current_user_token, exit_current_and_run_next, get_syscall_count,
    suspend_current_and_run_next, with_current_memory_set,
};

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
/// 获取当前时间，并将时间写入用户提供的 TimeVal。
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");

    let token = current_user_token();

    let addr = ts as usize;
    let len = core::mem::size_of::<TimeVal>();

    // 1. 检查用户地址是否可写
    if !check_user_buffer(token, addr, len, PTEFlags::W) {
        return -1;
    }

    // 2. 获取当前时间
    let time = get_time_us();

    let value = TimeVal {
        sec: time / 1_000_000,
        usec: time % 1_000_000,
    };

    // 3. 将 TimeVal 转换为字节切片
    let src = unsafe { core::slice::from_raw_parts(&value as *const TimeVal as *const u8, len) };

    // 4. 将用户虚拟地址转换为物理内存切片
    let buffers = translated_byte_buffer(token, ts as *const u8, len);

    // 5. 将时间数据写入用户空间
    let mut offset = 0;

    for buffer in buffers {
        let next = offset + buffer.len();

        buffer.copy_from_slice(&src[offset..next]);

        offset = next;
    }

    0
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
/// 用户态内存读取、写入和系统调用次数查询。
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");

    let token = current_user_token();

    match trace_request {
        // 0：读取用户地址中的一个字节
        0 => {
            if !check_user_buffer(token, id, 1, PTEFlags::R) {
                return -1;
            }

            let buffers = translated_byte_buffer(token, id as *const u8, 1);

            buffers[0][0] as isize
        }

        // 1：向用户地址写入一个字节
        1 => {
            if !check_user_buffer(token, id, 1, PTEFlags::W) {
                return -1;
            }

            let mut buffers = translated_byte_buffer(token, id as *const u8, 1);

            buffers[0][0] = data as u8;

            0
        }

        // 2：查询系统调用次数
        2 => get_syscall_count(id),

        _ => -1,
    }
}

// YOUR JOB: Implement mmap.
/// 匿名内存映射。
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!("kernel: sys_mmap");

    // Sv39 用户空间上界
    const USER_END: usize = 1usize << 38;

    // 1. 起始地址必须页对齐
    if start % PAGE_SIZE != 0 {
        return -1;
    }

    // 2. prot 只能使用最低三位
    if prot & !0x7 != 0 {
        return -1;
    }

    // 3. 至少具有一种权限
    if prot & 0x7 == 0 {
        return -1;
    }

    // 4. 起始地址必须位于用户空间
    if start >= USER_END {
        return -1;
    }

    // 5. 长度为 0 时，不需要分配页面
    if len == 0 {
        return 0;
    }

    // 6. 检查结束地址是否溢出
    let end = match start.checked_add(len) {
        Some(end) => end,
        None => return -1,
    };

    if end > USER_END {
        return -1;
    }

    // 7. 将用户 prot 转换成 MapPermission
    let mut permission = MapPermission::U;

    if prot & 0x1 != 0 {
        permission |= MapPermission::R;
    }

    if prot & 0x2 != 0 {
        permission |= MapPermission::W;
    }

    if prot & 0x4 != 0 {
        permission |= MapPermission::X;
    }

    // 8. 在当前任务的地址空间中建立映射
    let start_va = VirtAddr::from(start);
    let end_va = VirtAddr::from(end);

    let success = with_current_memory_set(|memory_set| {
        // 检查目标区域是否已经存在映射
        if !memory_set.is_range_unmapped(start_va, end_va) {
            return false;
        }

        // 尝试分配物理页帧并建立映射
        memory_set.try_insert_framed_area(start_va, end_va, permission)
    });

    if success {
        0
    } else {
        -1
    }
}

// YOUR JOB: Implement munmap.
/// 解除指定虚拟地址范围的映射。
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap");

    const USER_END: usize = 1usize << 38;

    // 1. 起始地址必须页对齐
    if start % PAGE_SIZE != 0 {
        return -1;
    }

    if start >= USER_END {
        return -1;
    }

    // 2. 长度为 0，不需要解除映射
    if len == 0 {
        return 0;
    }

    // 3. 检查地址计算是否溢出
    let end = match start.checked_add(len) {
        Some(end) => end,
        None => return -1,
    };

    if end > USER_END {
        return -1;
    }

    // 4. 获取当前任务的地址空间
    let start_va = VirtAddr::from(start);
    let end_va = VirtAddr::from(end);

    let success =
        with_current_memory_set(|memory_set| memory_set.unmap_framed_range(start_va, end_va));

    if success {
        0
    } else {
        -1
    }
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
