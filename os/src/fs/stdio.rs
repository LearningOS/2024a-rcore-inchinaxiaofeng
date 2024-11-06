//!Stdin & Stdout
use super::File;
use crate::mm::UserBuffer;
use crate::sbi::console_getchar;
use crate::task::suspend_current_and_run_next;

// NOTE: 第二章就对应用程序引入了基于**文件**的标准输出接口`sys_write`,在第五章引入标准输入接口`sys_read`.

/// `stdin` file for getting chars from console
/// Std In Device在文件描述符表中的文件描述符值为`01`
pub struct Stdin;

/// `stdout` file for putting chars to console
/// Std Out Device在文件描述符表中的文件描述符值为`1`
pub struct Stdout;

/// File Trait
impl File for Stdin {
    /// 只读文件
    fn readable(&self) -> bool {
        true
    }
    fn writable(&self) -> bool {
        false
    }
    /// `loop`每次仅支持读取1个字符,通过UserBuffer来获取具体将字节写入的位置
    fn read(&self, mut user_buf: UserBuffer) -> usize {
        assert_eq!(user_buf.len(), 1);
        // Busy loop
        let mut c: usize;
        loop {
            c = console_getchar();
            if c == 0 {
                suspend_current_and_run_next();
                continue;
            } else {
                break;
            }
        }
        let ch = c as u8;
        unsafe {
            user_buf.buffers[0].as_mut_ptr().write_volatile(ch);
        }
        1
    }
    fn write(&self, _user_buf: UserBuffer) -> usize {
        panic!("Cannot write to stdin!");
    }
}

impl File for Stdout {
    fn readable(&self) -> bool {
        false
    }
    /// 只写文件
    fn writable(&self) -> bool {
        true
    }
    fn read(&self, _user_buf: UserBuffer) -> usize {
        panic!("Cannot read from stdout!");
    }
    /// 遍历每个切片,将其转化为字符串通过`print!`宏来输出
    fn write(&self, user_buf: UserBuffer) -> usize {
        for buffer in user_buf.buffers.iter() {
            print!("{}", core::str::from_utf8(*buffer).unwrap());
        }
        user_buf.len()
    }
}
