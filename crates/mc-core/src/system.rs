//! 机器硬件探测 + 内存推荐启发式。
//!
//! 用户在「最大内存」滑块上常常瞎猜:给太少会卡顿/OOM,给太多会挤占系统。
//! 这里提供两件事:
//! - [`system_total_mem_mb`]:读取本机物理内存总量(MiB),用 `sysinfo`,有 I/O。
//! - [`suggest_memory_mb`]:**纯函数**启发式,按总内存 + mod 数量给出一个合理的堆上限,
//!   可单测、不触碰系统,便于在不同 (total, mod_count) 点位锁定行为。

/// 堆内存推荐的基准值(MiB):原版 / 无 mod 的安全起点。
const BASE_MB: u64 = 2048;
/// 一旦是「带 mod 的实例」额外加的基线(MiB):loader + 常驻 mod 的固定开销。
const MODDED_BASELINE_MB: u64 = 1536;
/// mod 数量每满一个「档」追加的内存(MiB)。
const PER_STEP_MB: u64 = 512;
/// 多少个 mod 算一档。
const MODS_PER_STEP: usize = 50;
/// 推荐值的硬上限(MiB):超过 8 GiB 对绝大多数整合包收益递减,且 G1 GC 在超大堆上更易抖动。
const HARD_CAP_MB: u64 = 8192;
/// 推荐值的常规下限(MiB)。仅当机器内存极小、60% 上限低于此值时才会被压破。
const FLOOR_MB: u64 = 2048;

/// 本机物理内存总量(MiB)。读取失败/为 0 时返回 0,由调用方决定如何兜底。
///
/// 只刷新内存信息(不枚举进程/磁盘/网络),开销很小。
pub fn system_total_mem_mb() -> u64 {
    use sysinfo::{MemoryRefreshKind, RefreshKind, System};
    let sys = System::new_with_specifics(
        RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    // sysinfo 自 0.30 起 `total_memory()` 返回字节。
    sys.total_memory() / 1024 / 1024
}

/// 依据物理内存总量与 mod 数量,给出一个合理的最大堆内存(MiB)。
///
/// 纯函数、无 I/O,便于单测。规则:
/// - 基线 [`BASE_MB`];带 mod 时再叠 [`MODDED_BASELINE_MB`],并按每 [`MODS_PER_STEP`] 个
///   mod 追加 [`PER_STEP_MB`]。
/// - 永不超过物理内存的 ~60%,也不超过 [`HARD_CAP_MB`](给系统/GC 留足余量)。
/// - 常规下限 [`FLOOR_MB`];但当机器内存极小、60% 上限已低于下限时,以 60% 上限为准
///   (避免在小内存机器上反而推荐一个会 OOM 的值)。
pub fn suggest_memory_mb(total_mb: u64, mod_count: usize) -> u32 {
    let mut suggested = BASE_MB;
    if mod_count > 0 {
        suggested += MODDED_BASELINE_MB;
        let steps = (mod_count / MODS_PER_STEP) as u64;
        suggested += steps * PER_STEP_MB;
    }

    // 不超过物理内存的 60%,也不超过硬上限。
    let upper = (total_mb.saturating_mul(6) / 10).min(HARD_CAP_MB);
    let suggested = suggested.min(upper);
    // 下限 FLOOR_MB,但绝不抬高到 60% 上限之上。
    let suggested = suggested.max(FLOOR_MB.min(upper));

    suggested as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_on_ample_ram_is_base() {
        // 16 GiB,无 mod:基线 2048,60% 上限远高于它。
        assert_eq!(suggest_memory_mb(16384, 0), 2048);
        // 32 GiB,无 mod:仍是基线(无 mod 不叠加)。
        assert_eq!(suggest_memory_mb(32768, 0), 2048);
    }

    #[test]
    fn modded_scales_with_count() {
        // 16 GiB,100 个 mod:2048 + 1536 + (100/50=2)*512 = 4608。
        assert_eq!(suggest_memory_mb(16384, 100), 4608);
        // 32 GiB,300 个 mod:2048 + 1536 + (6)*512 = 6656。
        assert_eq!(suggest_memory_mb(32768, 300), 6656);
        // 少量 mod(<50)只吃模组基线,不进档:2048 + 1536 = 3584。
        assert_eq!(suggest_memory_mb(16384, 10), 3584);
    }

    #[test]
    fn never_exceeds_hard_cap() {
        // 巨量 mod + 大内存:被 8192 硬上限钳住。
        assert_eq!(suggest_memory_mb(32768, 1000), 8192);
        assert_eq!(suggest_memory_mb(65536, 5000), 8192);
    }

    #[test]
    fn never_exceeds_60_percent_of_ram() {
        // 8 GiB,200 个 mod:启发式想要 2048+1536+4*512=5632,但 60% 上限 = 4915。
        assert_eq!(suggest_memory_mb(8192, 200), 4915);
        // 4 GiB,无 mod:基线 2048 已落在 60%(2457)以内。
        assert_eq!(suggest_memory_mb(4096, 0), 2048);
    }

    #[test]
    fn tiny_ram_respects_cap_over_floor() {
        // 2 GiB:60% = 1228,低于 2048 下限 —— 以 60% 上限为准,避免推荐会 OOM 的值。
        assert_eq!(suggest_memory_mb(2048, 50), 1228);
        // 3 GiB 无 mod:60% = 1843,仍低于下限,以上限为准。
        assert_eq!(suggest_memory_mb(3072, 0), 1843);
    }

    #[test]
    fn zero_total_is_safe() {
        // 探测失败(total=0):上限为 0,返回 0,不 panic;调用方负责兜底。
        assert_eq!(suggest_memory_mb(0, 0), 0);
    }

    #[test]
    fn detected_total_is_plausible_or_zero() {
        // 真实探测:CI 上至少有几百 MiB;允许 0(无 sysinfo 后端的极端环境)。
        let total = system_total_mem_mb();
        assert!(total == 0 || total > 256, "意外的内存总量: {total}");
    }
}
