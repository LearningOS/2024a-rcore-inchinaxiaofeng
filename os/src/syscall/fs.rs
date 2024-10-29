//! File and filesystem-related syscalls

use crate::mm::translated_byte_buffer;
use crate::task::current_user_token;

const FD_STDOUT: usize = 1;

// NOTE: 对sys_write进行改造
/// write buf of length `len`  to a file with `fd`
pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel: sys_write");
    match fd {
        FD_STDOUT => {
            let buffers = translated_byte_buffer(current_user_token(), buf, len);
            for buffer in buffers {
                // NOTE: 我们尝试将每个字节数组切片转化为字符串 &str 然后输出即可
                print!("{}", core::str::from_utf8(buffer).unwrap());
            }
            len as isize
        }
        _ => {
            panic!("Unsupported fd in sys_write!");
        }
    }
}
