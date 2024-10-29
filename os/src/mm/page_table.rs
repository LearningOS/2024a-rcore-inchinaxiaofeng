//! Implementation of [`PageTableEntry`] and [`PageTable`].

use super::{frame_alloc, FrameTracker, PhysPageNum, StepByOne, VirtAddr, VirtPageNum};
use alloc::vec;
use alloc::vec::Vec;
use bitflags::*;

// NOTE: 实现页表项中的标志位`PTEFlags`
// bitflags 是一个 Rust 中常用来比特标志位的 crate，提供了`bitflags!`宏
// 可以将`u8`封装成一个标志位的集合类型
bitflags! {
    /// page table entry flags
    pub struct PTEFlags: u8 {
        const V = 1 << 0;
        const R = 1 << 1;
        const W = 1 << 2;
        const X = 1 << 3;
        const U = 1 << 4;
        const G = 1 << 5;
        const A = 1 << 6;
        const D = 1 << 7;
    }
}

// NOTE: 我们让编译器自动为 PageTableEntry 实现 Copy/Clone Trait，
// 来让这个类型以值语义赋值/传参的时候不会发生所有权转移，而是拷贝一份新的副本。
// 从这一点来说PageTableEntry就和usize一样，
// 因为它也只是后者的一层简单包装，并解释了usize各个比特段的含义。
#[derive(Copy, Clone)]
#[repr(C)]
/// page table entry structure
pub struct PageTableEntry {
    /// bits of page table entry
    pub bits: usize,
}

impl PageTableEntry {
    // NOTE: 可以从一个物理页号 PhysPageNum 和一个页表项标志位 PTEFlags
    // 生成一个页表项 PageTableEntry 实例
    /// Create a new page table entry
    pub fn new(ppn: PhysPageNum, flags: PTEFlags) -> Self {
        PageTableEntry {
            bits: ppn.0 << 10 | flags.bits as usize,
        }
    }
    // NOTE: 我们也可以通过 empty 方法生成一个全零的页表项，
    // 注意这隐含着该页表项的 V 标志位为 0 ，因此它是不合法的
    /// Create an empty page table entry
    pub fn empty() -> Self {
        PageTableEntry { bits: 0 }
    }
    // NOTE: 实现了分别可以从一个页表项将它们两个取出的方法
    /// Get the physical page number from the page table entry
    pub fn ppn(&self) -> PhysPageNum {
        (self.bits >> 10 & ((1usize << 44) - 1)).into()
    }
    /// Get the flags from the page table entry
    pub fn flags(&self) -> PTEFlags {
        PTEFlags::from_bits(self.bits as u8).unwrap()
    }
    // NOTE: 辅助函数(Helper Function)
    // 可以快速判断一个页表项的 V/R/W/X 标志位是否为 1 以 V 标志位的判断为例
    /// The page pointered by page table entry is valid?
    pub fn is_valid(&self) -> bool {
        (self.flags() & PTEFlags::V) != PTEFlags::empty()
    }
    /// The page pointered by page table entry is readable?
    pub fn readable(&self) -> bool {
        (self.flags() & PTEFlags::R) != PTEFlags::empty()
    }
    /// The page pointered by page table entry is writable?
    pub fn writable(&self) -> bool {
        (self.flags() & PTEFlags::W) != PTEFlags::empty()
    }
    /// The page pointered by page table entry is executable?
    pub fn executable(&self) -> bool {
        (self.flags() & PTEFlags::X) != PTEFlags::empty()
    }
}

// NOTE: SV39 多级页表是以节点为单位进行管理的。
// 每个节点恰好存储在一个物理页帧中，它的位置可以用一个物理页号来表示
/// page table structure
pub struct PageTable {
    // NOTE: 不同页表之间，root_ppn是唯一的区分标志
    root_ppn: PhysPageNum,
    // NOTE: 将FrameTracker进一步绑定到所在的物理页帧
    // 生命周期结束后，frames里的FrameTracker就被回收
    frames: Vec<FrameTracker>,
}

// NOTE: 当遇到需要查一个特定页表（非当前正处在的地址空间的页表时），
// 便可先通过 PageTable::from_token 新建一个页表，
// 再调用它的 translate 方法查页表。
/// Assume that it won't oom when creating/mapping.
impl PageTable {
    /// Create a new page table
    pub fn new() -> Self {
        let frame = frame_alloc().unwrap();
        PageTable {
            root_ppn: frame.ppn, // NOTE: 一个页表只会有一个root节点
            frames: vec![frame],
        }
    }
    // NOTE: 临时创建一个专门用于手动查表的PageTable，
    // 它仅有一个从传入的 satp token 中得到的多级页表根节点的物理页号，
    // 它的 frames 字段为空，也即不实际控制任何资源；
    /// Temporarily used to get arguments from user space.
    pub fn from_token(satp: usize) -> Self {
        Self {
            root_ppn: PhysPageNum::from(satp & ((1usize << 44) - 1)),
            frames: Vec::new(),
        }
    }
    // NOTE: 多级页表找到一个虚拟页号对应的页表项的可变引用
    // 如果在遍历的过程中发现有节点尚未创建则会新建一个节点
    /// Find PageTableEntry by VirtPageNum, create a frame for a 4KB page table if not exist
    fn find_pte_create(&mut self, vpn: VirtPageNum) -> Option<&mut PageTableEntry> {
        let idxs = vpn.indexes();
        // NOTE: ppn表示当前节点的物理页号，最开始是多级页表的根节点
        let mut ppn = self.root_ppn;
        let mut result: Option<&mut PageTableEntry> = None;
        for (i, idx) in idxs.iter().enumerate() {
            // NOTE: 通过get_pte_array取出当前节点的页表项数组，
            // 根据当前级页索引找到对应的页表项
            let pte = &mut ppn.get_pte_array()[*idx];
            if i == 2 {
                // NOTE: 如果当前节点为叶节点，返回
                result = Some(pte);
                break;
            }
            // NOTE: 不是叶节点，继续往下走，不存在就创建新的节点
            if !pte.is_valid() {
                let frame = frame_alloc().unwrap();
                *pte = PageTableEntry::new(frame.ppn, PTEFlags::V);
                self.frames.push(frame);
            }
            ppn = pte.ppn();
        }
        result
    }
    // NOTE: 与find_pte_create不同是，不存在的时候直接返回None
    /// Find PageTableEntry by VirtPageNum
    fn find_pte(&self, vpn: VirtPageNum) -> Option<&mut PageTableEntry> {
        let idxs = vpn.indexes();
        let mut ppn = self.root_ppn;
        let mut result: Option<&mut PageTableEntry> = None;
        for (i, idx) in idxs.iter().enumerate() {
            let pte = &mut ppn.get_pte_array()[*idx];
            if i == 2 {
                result = Some(pte);
                break;
            }
            if !pte.is_valid() {
                return None;
            }
            ppn = pte.ppn();
        }
        result
    }
    // NOTE: 找到或创建
    /// set the map between virtual page number and physical page number
    #[allow(unused)]
    pub fn map(&mut self, vpn: VirtPageNum, ppn: PhysPageNum, flags: PTEFlags) {
        let pte = self.find_pte_create(vpn).unwrap();
        assert!(!pte.is_valid(), "vpn {:?} is mapped before mapping", vpn);
        *pte = PageTableEntry::new(ppn, flags | PTEFlags::V);
    }
    // NOTE:
    /// remove the map between virtual page number and physical page number
    #[allow(unused)]
    pub fn unmap(&mut self, vpn: VirtPageNum) {
        let pte = self.find_pte(vpn).unwrap();
        assert!(pte.is_valid(), "vpn {:?} is invalid before unmapping", vpn);
        *pte = PageTableEntry::empty();
    }
    // NOTE: 调用find_pte实现，能够找到就返回一个拷贝，找不到就None
    /// get the page table entry from the virtual page number
    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.find_pte(vpn).map(|pte| *pte)
    }
    // NOTE: 会按照satp CSR格式要求构造一个无符号64位无符号整数，
    // 使得其分页模式为SV39，且将当前多级页表的根节点所在的物理页号填充进去。
    /// get the token from the page table
    pub fn token(&self) -> usize {
        8usize << 60 | self.root_ppn.0
    }
}

// NOTE: 页表模块 page_table 提供了将应用地址空间中一个缓冲区转化为在内核空间中能够直接访问的形式的辅助函数
/// Translate&Copy a ptr[u8] array with LENGTH len to a mutable u8 Vec through page table
pub fn translated_byte_buffer(
    token: usize,   // NOTE: 某个应用地址空间的token
    ptr: *const u8, // NOTE: 该地址空间中的一段缓冲区起始地址
    len: usize,     // NOTE: 该地址空间中的一段缓冲区的长度
                    // NOTE: 以向量的形式返回一组可以在内核空间中直接访问的字节数组切片
) -> Vec<&'static mut [u8]> {
    let page_table = PageTable::from_token(token);
    let mut start = ptr as usize;
    let end = start + len;
    let mut v = Vec::new();
    while start < end {
        let start_va = VirtAddr::from(start);
        let mut vpn = start_va.floor();
        let ppn = page_table.translate(vpn).unwrap().ppn();
        vpn.step();
        let mut end_va: VirtAddr = vpn.into();
        end_va = end_va.min(VirtAddr::from(end));
        if end_va.page_offset() == 0 {
            v.push(&mut ppn.get_bytes_array()[start_va.page_offset()..]);
        } else {
            v.push(&mut ppn.get_bytes_array()[start_va.page_offset()..end_va.page_offset()]);
        }
        start = end_va.into();
    }
    v
}
