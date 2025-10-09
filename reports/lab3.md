## 功能实现总结

本次实验在原有基于优先级调度的基础上实现了 **Stride 调度算法**。该算法为每个任务分配一个 stride 值，stride 越小优先级越高，每次调度后将该任务的 stride 增加一个与优先级相关的步长。这样可以在长时间内实现与比例调度相近的公平性。本实验还将 stride、pass、priority 三个参数加入任务控制块，并在调度器中使用最小堆选择 stride 最小的任务，从而实现动态、可控的任务分配。

## 问答作业

### 问题1

> stride 算法原理非常简单，但是有一个比较大的问题。例如两个 pass = 10 的进程，使用 8bit 无符号整形储存 stride， p1.stride = 255, p2.stride = 250，在 p2 执行一个时间片后，理论上下一次应该 p1 执行。
> - 实际情况是轮到 p1 执行吗？为什么？

不会。因为 8bit 无符号整数溢出后会回到 0。
 当 p2 执行一次时间片后，若它的 stride 增加 10，则
 p2.stride = (250 + 10) % 256 = 4。
 此时调度器比较 stride 值时，会认为 4 < 255，因此仍然认为 p2 的 stride 更小，于是继续执行 p2。
 也就是说，**由于溢出导致比较逻辑错误，调度器错误地优先执行了 stride 溢出的进程。**

### 问题2

> 我们之前要求进程优先级 >= 2 其实就是为了解决这个问题。可以证明， **在不考虑溢出的情况下** , 在进程优先级全部 >= 2 的情况下，如果严格按照算法执行，那么 STRIDE_MAX – STRIDE_MIN <= BigStride / 2。
>
> - 为什么？尝试简单说明（不要求严格证明）。

因为 stride 的增长步长为 `BigStride / priority`。
 当所有 priority ≥ 2 时，每次 stride 增量 ≤ BigStride / 2。
 这意味着在系统中任意进程间 stride 值的最大差距，不会超过 BigStride / 2。
 即使有一个进程连续运行，也无法让 stride 差距超过 BigStride / 2。
 这样在溢出发生时（例如从255回到0），仍能通过特殊比较器正确判断“较小”的 stride，而不会出现歧义。
 换言之，**约束 priority ≥ 2 保证 stride 值的差距不跨越溢出中点，从而避免比较混乱。**

### 问题3

> - 已知以上结论，**考虑溢出的情况下**，可以为 Stride 设计特别的比较器，让 BinaryHeap<Stride> 的 pop 方法能返回真正最小的 Stride。补全下列代码中的 `partial_cmp` 函数，假设两个 Stride 永远不会相等。

```rust
use core::cmp::Ordering;

const BIG_STRIDE: u64 = 255;

struct Stride(u64);

impl PartialOrd for Stride {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // 计算差值
        let diff = self.0.wrapping_sub(other.0);
        // 若差值在 [0, BIG_STRIDE/2) 内，说明 self > other
        // 若差值在 [BIG_STRIDE/2, BIG_STRIDE) 内，说明 self < other（溢出）
        if diff == 0 {
            None
        } else if diff < BIG_STRIDE / 2 {
            Some(Ordering::Greater)
        } else {
            Some(Ordering::Less)
        }
    }
}

impl PartialEq for Stride {
    fn eq(&self, other: &Self) -> bool {
        false
    }
}

```

## 荣誉准则

1. 在完成本次实验的过程（含此前学习的过程）中，我曾分别与 **以下各位** 就（与本次实验相关的）以下方面做过交流，还在代码中对应的位置以注释形式记录了具体的交流对象及内容：

   > 无

2. 此外，我也参考了 **以下资料** ，还在代码中对应的位置以注释形式记录了具体的参考来源及内容：

   > 无

3. 我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。 我清楚地知道，从以上方面获得的信息在一定程度上降低了实验难度，可能会影响起评分。

4. 我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。 我未曾也不会向他人（含此后各届同学）复制或公开我的实验代码，我有义务妥善保管好它们。 我提交至本实验的评测系统的代码，均无意于破坏或妨碍任何计算机系统的正常运转。 我清楚地知道，以上情况均为本课程纪律所禁止，若违反，对应的实验成绩将按“-100”分计。
