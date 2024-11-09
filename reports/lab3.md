# 功能总结

首先，我们重新实现了以下系统调用：

* `sys_get_time`
  * 这个内容与CH4是一致的
* `sys_task_info`
  * 使用了新的函数进行封装，提供了更好的结构，目前使用的新的函数：
    * `get_current_task_status()`
    * `get_current_task_syscall_times()`
    * `get_current_task_time_cost()`
  * 然后返回task_info
* `sys_mmap`
  * 与之前的内容相似
* `sys_munmap`
  * 与之前的内容相似

其次，我们实现了新的函数：

* sys_spawn
  * 这个函数我参考了fork函数，通过pid_alloc分配新的pid,并且自己指定TCB的创建（与fork函数相同）
  * 然后直接将新的Process压入父进程的children列表
  * 创造出trap_cx并返回pid
* sys_get_priority
  * 调用set_priority函数来指定
  * 在fetch中实现了stride策略

# 问答题

## stride 算法深入

stride 算法原理非常简单，但是有一个比较大的问题。例如两个 pass = 10 的进程，使用 8bit 无符号整形储存 stride， p1.stride = 255, p2.stride = 250，在 p2 执行一个时间片后，理论上下一次应该 p1 执行。

* 实际情况是轮到 p1 执行吗？为什么？

> 在使用 8 位无符号整型存储 stride 的情况下，当 p1 的 stride 为 255，p2 的 stride 为 250，并且 p2 执行了一个时间片后，p2 的 stride 会增加 pass（假设 pass 为 10），因此 p2 的 stride 会变为 260。然而，由于 stride 是使用 8 位存储的，这将导致 p2 的 stride 回绕到 4（因为 260 % 256 = 4）。
>
> 此时，p1 依然是 255，而 p2 变成了 4。因此，下一次调度时，p2 的 stride 小于 p1 的 stride，所以实际上会选择 p2 而不是 p1 执行。这就是为什么在这种情况下 p1 并不会被执行。
>
> 我们之前要求进程优先级 >= 2 其实就是为了解决这个问题。可以证明， 在不考虑溢出的情况下 , 在进程优先级全部 >= 2 的情况下，如果严格按照算法执行，那么 STRIDE_MAX – STRIDE_MIN <= BigStride / 2。

* 为什么？尝试简单说明（不要求严格证明）。

> 要求进程的优先级大于或等于 2 主要是为了确保 pass 的值不会过小。因为 pass 的计算公式为：pass = BIG_STRIDE/ priority。
> 当优先级为 2 时，pass 的最大值为BIG_STRIDE/2。
> 这意味着进程的 stride 增加的速度受到限制，从而减少了频繁的溢出发生。具体来说：
>
> * 控制增量：保持优先级 >= 2 可以确保 pass 的值是一个适中的数值，避免 stride 在短时间内快速增加。
> * 维护范围：在理想情况下，这会导致 STRIDE_MAX - STRIDE_MIN 的差值被限制在 BIG_STRIDE/2 之内，确保调度的公平性，并减少溢出的可能性。

* 已知以上结论，考虑溢出的情况下，可以为 Stride 设计特别的比较器，让 BinaryHeap<Stride> 的 pop 方法能返回真正最小的 Stride。补全下列代码中的`partial_cmp`函数，假设两个 Stride 永远不会相等。

```Rust
use core::cmp::Ordering;

struct Stride(u64);

impl PartialOrd for Stride {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some((self.0 as i64).cmp(&(other.0 as i64)))
    }
}

impl PartialEq for Stride {
    fn eq(&self, other: &Self) -> bool {
        false
    }
}
```

TIPS: 使用 8 bits 存储 stride, BigStride = 255, 则:`(125 < 255) == false`, `(129 < 255) == true`.

# 荣誉准则

1. 在完成本次实验的过程（含此前学习的过程）中，我曾分别与 以下各位 就（与本次实验相关的）以下方面做过交流，还在代码中对应的位置以注释形式记录了具体的交流对象及内容：

2. 此外，我也参考了 以下资料 ，还在代码中对应的位置以注释形式记录了具体的参考来源及内容：

3. 我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。 我清楚地知道，从以上方面获得的信息在一定程度上降低了实验难度，可能会影响起评分。

4. 我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。 我未曾也不会向他人（含此后各届同学）复制或公开我的实验代码，我有义务妥善保管好它们。 我提交至本实验的评测系统的代码，均无意于破坏或妨碍任何计算机系统的正常运转。 我清楚地知道，以上情况均为本课程纪律所禁止，若违反，对应的实验成绩将按“-100”分计。
