//! Loading user applications into memory

// NOTE: 我们不再需要丢弃所有符号的应用二进制镜像链接进内核，而是直接使用ELF格式的可执行文件

// NOTE: 获取链接到内核内的应用的数目
/// Get the total number of applications.
pub fn get_num_app() -> usize {
    extern "C" {
        fn _num_app();
    }
    unsafe { (_num_app as usize as *const usize).read_volatile() }
}

// NOTE: 根据传入的应用编号 取出对应应用的 ELF 格式可执行文件数据
// 找到各个逻辑段所在位置和访问限制并插入进来，
// 最终得到一个完整的应用地址空间：
/// get applications data
pub fn get_app_data(app_id: usize) -> &'static [u8] {
    extern "C" {
        fn _num_app();
    }
    let num_app_ptr = _num_app as usize as *const usize;
    let num_app = get_num_app();
    let app_start = unsafe { core::slice::from_raw_parts(num_app_ptr.add(1), num_app + 1) };
    assert!(app_id < num_app);
    unsafe {
        core::slice::from_raw_parts(
            app_start[app_id] as *const u8,
            app_start[app_id + 1] - app_start[app_id],
        )
    }
}
