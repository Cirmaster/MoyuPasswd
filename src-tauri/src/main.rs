// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    harden_heap();
    app_lib::run();
}

/// 让 glibc 在 `free()` 时用固定字节覆写整块内存
///
/// # 为什么需要
///
/// 拖放交付时明文会经过三处：
/// 1. 我们自己解密出来的 `String` —— 已用 `Zeroizing` 自行清零；
/// 2. **GDK 的 `StoredSelection.data`** —— `gtk_selection_data_set()` 会 `g_memdup`
///    一份，拖放结束（`dnd_finished` → `gdk_selection_owner_set(NULL, ATOM_DND)` →
///    `reset_selection` → `stored_selection_free`）时才 `g_free`，且**不清零**；
/// 3. 目标进程里的副本 —— 不归我们管。
///
/// 第 2 处拿不到指针：`StoredSelection` 是 GDK 私有结构体，而 `GtkSelectionData`
/// 在 `drag-data-get` 回调返回后就被释放（`gtkselection.c` 里），拖放结束时去写它是
/// use-after-free。所以只能让分配器代劳：`mallopt(M_PERTURB, v)` 之后，
/// glibc 每次 free 都会把块填成固定字节，被释放的明文随之中止可读。
///
/// 代价：每次 malloc/free 多一次 memset。本应用分配频率很低，可以忽略。
#[cfg(target_os = "linux")]
fn harden_heap() {
    /// glibc `<malloc.h>` 的 `M_PERTURB`（-6）。非 POSIX，musl 没有这个接口。
    const M_PERTURB: i32 = -6;
    /// 覆写用的字节值，只要非 0 即可（0 表示关闭该特性）
    const FILL: i32 = 0x5a;

    extern "C" {
        fn mallopt(param: i32, value: i32) -> i32;
    }

    // SAFETY: mallopt 只改 glibc 的全局分配参数，内部有锁，任何时刻调用都安全；
    // 参数是编译期常量，不会越界。
    let ok = unsafe { mallopt(M_PERTURB, FILL) };
    if ok == 0 {
        // 非 glibc（musl）或参数不被支持：功能不受影响，只是少了这层覆盖
        eprintln!("[HARDEN] 分配器不支持 M_PERTURB，已释放内存不会被自动覆写");
    }
}

#[cfg(not(target_os = "linux"))]
fn harden_heap() {}
