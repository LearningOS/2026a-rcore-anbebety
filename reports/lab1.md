# rCore 操作系统实验报告：用户态异常与 Trap 上下文切换

## 一、实验功能总结（200字以内）

本实验基于 rCore 第三章多道程序内核，实现并验证用户态异常处理机制。通过运行非法指令、非法访问 S 态寄存器和空地址写入三个 bad 测例，观察内核捕获异常并终止对应应用的过程。同时，通过分析 `trap.S` 中的 `__alltraps` 和 `__restore`，理解用户态与内核态之间的上下文保存、恢复、栈切换以及特权级切换机制。

## 二、实验环境

| 项目 | 配置 |
|---|---|
| 操作系统 | Ubuntu（WSL） |
| 处理器架构 | RISC-V 64 |
| 模拟器 | QEMU virt |
| 内核 | rCore Tutorial，ch3 分支 |
| SBI | RustSBI 0.4.1 |
| Rust 工具链 | nightly-2024-05-02 |

说明：本实验使用仓库提供的 `bootloader/rustsbi-qemu.bin` 固件，`os/Makefile` 默认使用 RustSBI。仓库的 `bootloader/rustsbi-qemu.toml` 明确标注固件版本为 0.4.1。课程 Docker 和评测环境使用 QEMU 8.2.2，本地 QEMU 版本以实际安装情况为准。

## 三、问答题

### 1. 运行三个 bad 测例，描述程序出错行为

本实验通过运行三个错误测试程序，验证用户态程序不能随意执行 S 态特权指令、访问 S 态寄存器以及非法访问内存。

**（1）ch2b_bad_instructions.rs：执行 S 态特权指令**

该程序通过内联汇编执行 `sret` 指令。`sret` 是 RISC-V 的 Supervisor Return 指令，用于从 S 态返回之前的特权级。由于应用程序当前运行在 U 态，不具备执行该指令的权限，因此执行时会触发非法指令异常（IllegalInstruction）。

内核通过 `trap_handler` 捕获异常，输出：

```text
[kernel] IllegalInstruction in application, kernel killed it.
```

随后调用 `exit_current_and_run_next()` 终止当前应用并切换到下一个任务。由于发生异常，程序后面的 `panic!` 不会执行。

**（2）ch2b_bad_register.rs：访问 S 态寄存器**

该程序使用 `csrr` 指令读取 `sstatus` 寄存器。`sstatus` 是 Supervisor Status Register，属于 S 态控制状态寄存器。

由于当前程序运行在 U 态，没有权限直接读取 S 态寄存器，因此执行该指令时会触发非法指令异常（IllegalInstruction）。

内核捕获异常后，同样输出：

```text
[kernel] IllegalInstruction in application, kernel killed it.
```

随后终止当前应用，调度下一个任务。

**（3）ch2b_bad_address.rs：非法访问内存地址**

该程序尝试向地址 `0x0` 写入数据：

```rust
(0x0 as *mut u8).write_volatile(0);
```

由于地址 0 不属于该应用合法使用的内存区域，执行写操作时会触发存储访问异常，具体可能表现为 Store/AMO Access Fault 或 Store/AMO Page Fault，取决于实际内存映射和地址转换配置。

在当前 ch3 内核中，`trap_handler` 对 `StoreFault` 和 `StorePageFault` 两种异常进行了统一处理，输出：

```text
[kernel] PageFault in application, bad addr = 0x0, bad instruction = 0x..., kernel killed it.
```

其中，`bad addr` 表示发生错误的内存地址，`bad instruction` 表示发生异常时的指令地址。随后内核终止当前应用并运行下一个任务。

**实验现象总结：**

三个 bad 测例分别验证了 U 态对特权指令、特权寄存器和非法内存访问的限制。发生异常后，处理器进入 S 态，内核通过 `trap_handler` 判断异常类型，并终止发生错误的应用，而不会让错误程序继续执行。

需要注意，测试程序源码中包含关于旧版 RustSBI 退出问题的注释，标明 RustSBI 0.2.0-alpha.1 已修复该问题。本实验使用 RustSBI 0.4.1，已经高于该修复版本。

### 2. 深入理解 trap.S 中 __alltraps 和 __restore 的作用

#### 2.1 __alltraps 和 __restore 的整体作用

在 rCore 中，`__alltraps` 和 `__restore` 是用户态与内核态进行上下文切换的两个关键汇编函数。

**__alltraps 的作用：**

当用户程序发生系统调用、异常或者中断时，处理器进入 S 态，并根据 `stvec` 中保存的地址跳转到 `__alltraps`。

该函数主要完成以下工作：

1. 交换用户栈和内核栈，切换到内核栈。
2. 在内核栈上分配 TrapContext 所需空间。
3. 保存用户程序的通用寄存器。
4. 保存 `sstatus` 和 `sepc` 等关键状态信息。
5. 保存用户态的栈指针。
6. 将 TrapContext 地址作为参数传递给 `trap_handler`。
7. 调用内核异常处理函数。

**__restore 的作用：**

当内核完成异常处理或系统调用处理后，需要恢复用户程序原来的执行现场。

该函数主要完成以下工作：

1. 从 TrapContext 中恢复 `sstatus`、`sepc` 等状态信息。
2. 恢复用户程序的通用寄存器。
3. 恢复用户态栈指针。
4. 释放内核栈上保存的 TrapContext。
5. 交换用户栈与内核栈。
6. 执行 `sret`，返回用户态继续执行。

两者共同完成了用户态与内核态之间的上下文保存和恢复。

#### 2.2 L40：刚进入 __restore 时，sp 代表什么？有哪两种使用情景？

刚进入 `__restore` 时，`sp` 指向内核栈中保存的 TrapContext 结构体的起始地址，也就是当前需要恢复的上下文所在位置。

TrapContext 的结构如下：

```rust
pub struct TrapContext {
    pub x: [usize; 32],
    pub sstatus: Sstatus,
    pub sepc: usize,
}
```

其中：

- `x[32]`：保存 32 个通用寄存器。
- `sstatus`：保存 Supervisor 状态寄存器。
- `sepc`：保存异常发生时的程序计数器。

`__restore` 有两种典型使用情景：

**第一种：应用程序第一次进入 U 态。**

内核初始化应用程序时，会在该应用的内核栈中构造一个初始 TrapContext，其中包含应用程序入口地址、用户栈指针以及初始 `sstatus` 等信息。

随后，`TaskContext::goto_restore()` 将 `__restore` 设置为任务切换后的返回地址，并将 TrapContext 地址设置为内核栈指针。

当任务第一次被调度时，`__switch` 恢复任务上下文并执行 `ret`，最终进入 `__restore`，通过恢复初始上下文使应用程序进入 U 态。

**第二种：应用程序执行完系统调用或异常处理后返回 U 态。**

当用户程序发生异常或执行系统调用时，`__alltraps` 保存当前上下文并调用 `trap_handler`。

处理完成后，程序执行到 `__restore`，恢复之前保存的用户上下文。如果发生了任务切换，则会先恢复对应任务的内核执行现场，再继续执行异常处理函数并进入 `__restore`。

因此，`__restore` 既负责应用程序第一次进入 U 态，也负责应用程序在系统调用、中断或异常处理完成后重新返回 U 态。

#### 2.3 L43-L48：特殊处理了哪些寄存器？这些寄存器有什么意义？

对应汇编代码：

```assembly
ld t0, 32*8(sp)
ld t1, 33*8(sp)
ld t2, 2*8(sp)
csrw sstatus, t0
csrw sepc, t1
csrw sscratch, t2
```

这几行代码特殊处理了三个寄存器：`sstatus`、`sepc` 和 `sscratch`。

**（1）sstatus：Supervisor Status Register**

```assembly
ld t0, 32*8(sp)
csrw sstatus, t0
```

`sstatus` 用于保存和控制 S 态的处理器状态，其中与特权级切换关系最密切的是：

- `SPP`：记录进入异常之前的特权级。
- `SPIE`：保存进入异常之前的中断使能状态。
- `SIE`：控制 S 态中断使能。

执行 `sret` 时，处理器会根据 `SPP` 确定返回的特权级。

如果 `SPP=0`，表示返回 U 态；如果 `SPP=1`，表示返回 S 态。

因此，恢复 `sstatus` 是正确返回原特权级的重要前提。

在应用程序首次进入 U 态时，`app_init_context()` 会通过 `sstatus.set_spp(SPP::User)` 将 `SPP` 设置为 User，保证后续执行 `sret` 时能够进入 U 态。

**（2）sepc：Supervisor Exception Program Counter**

```assembly
ld t1, 33*8(sp)
csrw sepc, t1
```

`sepc` 用于保存异常发生时的程序计数器，即异常发生时的指令地址。

在异常处理完成后，需要恢复 `sepc`，使处理器知道应该从哪里继续执行。

例如，用户程序执行 `ecall` 进入内核时，`sepc` 保存的是 `ecall` 指令的地址。内核在处理系统调用时执行：

```rust
cx.sepc += 4;
```

这是因为 RISC-V 中 `ecall` 指令长度为 4 字节，需要将返回地址移动到下一条指令，避免返回用户态后再次执行同一个 `ecall`。

**（3）sscratch：Supervisor Scratch Register**

```assembly
ld t2, 2*8(sp)
csrw sscratch, t2
```

`sscratch` 是 S 态提供的临时寄存器，在本实验中主要用于保存和交换用户栈与内核栈的指针。

在进入内核时，`__alltraps` 会通过 `sscratch` 获取用户态的栈指针，并将其保存到 TrapContext 中。

返回用户态时，需要先恢复用户栈指针到 `sscratch`，再通过 `csrrw` 指令交换 `sp` 和 `sscratch`，最终恢复用户态栈。

**总结：**

这三组寄存器分别负责恢复处理器状态、恢复程序执行位置和恢复栈切换所需的信息，是正确返回用户态的关键。

#### 2.4 L50-L56：为何跳过了 x2 和 x4？

对应代码：

```assembly
ld x1, 1*8(sp)
ld x3, 3*8(sp)
.set n, 5
.rept 27
    LOAD_GP %n
    .set n, n+1
.endr
```

这里没有通过普通方式恢复 `x2` 和 `x4`，主要原因如下。

**（1）跳过 x2：sp 栈指针寄存器**

`x2` 就是 `sp`，即 Stack Pointer。

它是一个特殊的通用寄存器，用于指向当前栈顶。

在恢复其他寄存器时，`sp` 仍然需要指向内核栈上的 TrapContext。如果过早将 `sp` 恢复为用户栈指针，就无法继续通过内核栈访问尚未恢复的寄存器。

因此，内核没有直接通过 `LOAD_GP` 恢复 `x2`，而是采取特殊处理：

1. 在 `__alltraps` 中，通过 `sscratch` 获取用户态 `sp`。
2. 将用户态 `sp` 保存到 TrapContext 的 `x[2]` 位置。
3. 在 `__restore` 中读取该值，并写入 `sscratch`。
4. 完成其他寄存器恢复后，释放 TrapContext。
5. 最后通过 `csrrw sp, sscratch, sp` 恢复用户态 `sp`。

这种方式可以保证内核在恢复上下文的整个过程中始终能够正常访问内核栈。

**（2）跳过 x4：tp 线程指针寄存器**

`x4` 对应 `tp`（Thread Pointer），通常用于线程局部存储（TLS）等功能。

在当前 rCore 实验中，用户应用程序不使用 `tp`，因此没有必要保存和恢复该寄存器。

如果未来操作系统支持线程局部存储或用户程序需要使用 `tp`，就必须考虑对它进行上下文保存和恢复，否则可能出现不同任务之间寄存器状态相互影响的问题。

#### 2.5 L60：执行 csrrw sp, sscratch, sp 后，sp 和 sscratch 分别有什么意义？

对应指令：

```assembly
csrrw sp, sscratch, sp
```

`csrrw` 是 RISC-V 的 CSR 读写交换指令。

其作用是将 `sscratch` 中的值写入 `sp`，同时将原来的 `sp` 写入 `sscratch`。

执行该指令之前：

- `sp`：指向内核栈栈顶。此时 TrapContext 已经恢复完毕，并且内核栈上的 TrapContext 空间已经释放。
- `sscratch`：保存用户态栈指针。

执行该指令之后：

- `sp`：指向用户态栈顶，用户栈恢复完成。
- `sscratch`：指向内核栈栈顶，为下一次进入 S 态时切换到内核栈做好准备。

因此，该指令完成了从内核栈到用户栈的切换，为后续执行 `sret` 返回 U 态做好准备。

#### 2.6 __restore 中发生状态切换的是哪一条指令？为何执行后会进入用户态？

真正完成特权级切换的指令是：

```assembly
sret
```

`sret` 是 RISC-V 的 Supervisor Return 指令，用于从 S 态返回之前的特权级。

执行 `sret` 时，处理器主要完成以下操作：

1. 根据 `sstatus.SPP` 确定返回的特权级。
2. 将 `sepc` 中保存的地址恢复到程序计数器 `PC`。
3. 将 `sstatus.SPIE` 的值恢复到 `sstatus.SIE`。
4. 将 `SPIE` 设置为 1，并将 `SPP` 重置为 U 态对应的状态。

在本实验中，初始 TrapContext 中已经将 `SPP` 设置为 `SPP::User`，因此执行 `sret` 后，处理器从 S 态切换到 U 态，并从 `sepc` 指定的位置继续执行用户程序。

需要注意，前面的 `csrrw` 只完成栈指针交换，并没有改变处理器的特权级。真正完成特权级切换的是 `sret`。

#### 2.7 L13：执行 csrrw sp, sscratch, sp 后，sp 和 sscratch 分别有什么意义？

对应指令：

```assembly
csrrw sp, sscratch, sp
```

该指令位于 `__alltraps` 的入口处。

当用户程序发生异常、中断或系统调用时，处理器会自动从 U 态切换到 S 态，并跳转到 `__alltraps`。

但是，硬件不会自动将用户态栈切换为内核栈，因此需要通过这条指令完成栈指针交换。

在进入 `__alltraps`、执行该指令之前：

- `sp`：保存用户态栈指针。
- `sscratch`：保存内核栈栈顶指针。

执行该指令之后：

- `sp`：获得内核栈栈顶指针，后续内核异常处理代码可以使用内核栈。
- `sscratch`：保存原来的用户态栈指针，方便后续将其保存到 TrapContext 中。

随后执行：

```assembly
addi sp, sp, -34*8
```

在内核栈上分配 34 个 8 字节的空间，用于保存完整的 TrapContext。

**总结：**

`__alltraps` 中的 `csrrw` 实现了用户栈到内核栈的切换，而 `__restore` 中的 `csrrw` 实现了内核栈到用户栈的切换。两者配合实现了用户态和内核态之间的栈切换。

#### 2.8 从 U 态进入 S 态是哪一条指令发生的？

在本实验的系统调用场景中，从 U 态进入 S 态是通过执行：

```assembly
ecall
```

实现的。

`ecall` 是 Environment Call 指令，用于主动请求更高特权级的软件服务。

当用户程序在 U 态执行 `ecall` 时，处理器会自动完成以下操作：

1. 将当前程序计数器保存到 `sepc`。
2. 将异常前的特权级保存到 `sstatus.SPP`。
3. 保存中断使能状态到 `sstatus.SPIE`。
4. 清除当前 S 态中断使能位 `SIE`。
5. 将处理器特权级切换到 S 态。
6. 根据 `stvec` 中的地址跳转到异常处理入口 `__alltraps`。

随后由 `__alltraps` 保存用户上下文，并调用 `trap_handler` 处理系统调用。

需要注意，`ecall` 是系统调用进入 S 态的指令，并不是所有进入 S 态的情况都由 `ecall` 触发。例如，执行非法指令、非法访问寄存器或者发生内存访问异常时，也会由处理器自动触发异常并进入 S 态；定时器中断则通过硬件中断机制进入 S 态。

## 四、实验总结

通过本次实验，理解了 RISC-V 处理器的特权级机制，以及用户态程序执行特权指令、访问特权寄存器和非法访问内存时的异常处理过程。

通过分析 `__alltraps` 和 `__restore`，进一步掌握了 TrapContext 的保存与恢复过程，理解了 `sstatus`、`sepc`、`sscratch` 等寄存器的作用，以及 `sp` 与 `sscratch` 交换实现双栈切换的原理。

最终明确了 `ecall` 触发系统调用进入内核、`sret` 完成从内核态返回用户态的过程，加深了对操作系统用户态与内核态切换机制的理解。

## 五、参考资料

1. [rCore-Tutorial-Test：三个 bad 测试程序](https://github.com/LearningOS/rCore-Tutorial-Test/tree/main/src/bin)
2. [rCore-Tutorial-Code：trap.S](https://github.com/LearningOS/rCore-Tutorial-Code/blob/ch3/os/src/trap/trap.S)
3. [rCore ch3：TrapContext 定义](https://github.com/LearningOS/2026a-rcore-anbebety/blob/ch3/os/src/trap/context.rs)
4. [rCore ch3：trap_handler 实现](https://github.com/LearningOS/2026a-rcore-anbebety/blob/ch3/os/src/trap/mod.rs)
5. [RustSBI 固件版本配置](https://github.com/LearningOS/2026a-rcore-anbebety/blob/ch3/bootloader/rustsbi-qemu.toml)