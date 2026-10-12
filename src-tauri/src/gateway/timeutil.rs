//! 时间工具：纪元毫秒、公历→日序、本地时间。
//!
//! 不引入 chrono/time 依赖：网关只需要三种能力，自己实现反而更可控，
//! 而且"次日本地 04:00"这类本地时区计算在 Windows 上直接问系统最准。

use std::time::{SystemTime, UNIX_EPOCH};

/// 当前纪元毫秒（UTC）。全仓统一用它，避免各处自己 `SystemTime::now()` 后比较口径不同。
/// 本地日期与小时 `(年, 月, 日, 时)` —— 每日动作要按**本地**日期判断"今天做没做"。
#[cfg(windows)]
pub fn local_date_hour() -> (i32, u32, u32, u32) {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: GetLocalTime 只读系统时间，无指针参数。
    let local = unsafe { GetLocalTime() };
    (
        local.wYear as i32,
        local.wMonth as u32,
        local.wDay as u32,
        local.wHour as u32,
    )
}

#[cfg(not(windows))]
pub fn local_date_hour() -> (i32, u32, u32, u32) {
    // 非 Windows：用 UTC 天数换算，够定时器判断日期用。
    let days = (now_ms() / 86_400_000) as i64;
    let (year, month, day) = civil_from_days(days);
    let hour = ((now_ms() / 3_600_000) % 24) as u32;
    (year as i32, month as u32, day as u32, hour)
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Howard Hinnant 的 days_from_civil：1970-01-01 起的天数（可为负）。
pub fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// 把「某时区的 Y-M-D H:M:S」换算成纪元毫秒。
pub fn civil_to_epoch_ms(
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    offset_secs: i64,
) -> u64 {
    let days = days_from_civil(year, month, day);
    let secs = days * 86_400 + hour * 3600 + minute * 60 + second - offset_secs;
    secs.max(0) as u64 * 1000
}

/// 下一个「本机时区 04:00」的纪元毫秒（余额停放用：避开北京时间凌晨的结算点）。
#[cfg(windows)]
pub fn next_local_4am_ms(now_ms: u64) -> u64 {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: GetLocalTime 只读系统时间，无指针参数，总是成功。
    let local = unsafe { GetLocalTime() };
    let (year, month, day) = (local.wYear as i64, local.wMonth as i64, local.wDay as i64);
    let local_now_secs = days_from_civil(year, month, day) * 86_400
        + local.wHour as i64 * 3600
        + local.wMinute as i64 * 60
        + local.wSecond as i64;
    // 本机时区偏移：同一瞬间「本地解释」与「UTC 解释」之差。
    let offset_secs = local_now_secs - (now_ms / 1000) as i64;
    let now_secs = (now_ms / 1000) as i64;
    let mut target = days_from_civil(year, month, day) * 86_400 + 4 * 3600 - offset_secs;
    if target <= now_secs {
        target += 86_400;
    }
    // 时钟回拨或时区跳变的极端情况下，至少停放一分钟，避免立刻重试打成死循环。
    (target.max(now_secs + 60)) as u64 * 1000
}

#[cfg(not(windows))]
pub fn next_local_4am_ms(now_ms: u64) -> u64 {
    // 非 Windows 平台没有等价的零依赖本地时区接口；粗粒度停放 12 小时。
    now_ms + 12 * 3600 * 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_days_match_unix_epoch() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1970, 1, 2), 1);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        // 2036-01-01 = 1970 起 66 年 + 16 个闰日
        assert_eq!(days_from_civil(2036, 1, 1), 66 * 365 + 16);
    }

    #[test]
    fn epoch_conversion_honours_timezone_offset() {
        // UTC+8 的 1970-01-01 08:00:00 就是 0。
        assert_eq!(civil_to_epoch_ms(1970, 1, 1, 8, 0, 0, 8 * 3600), 0);
        // 同一时刻换 UTC+0 表示则是同一天 00:00:00。
        assert_eq!(civil_to_epoch_ms(1970, 1, 1, 0, 0, 0, 0), 0);
        assert_eq!(civil_to_epoch_ms(1970, 1, 2, 0, 0, 0, 0), 86_400_000);
    }

    #[test]
    fn balance_parking_is_always_in_the_future() {
        let now = now_ms();
        let target = next_local_4am_ms(now);
        assert!(target > now, "停放时刻必须在将来：{target} <= {now}");
        assert!(target <= now + 25 * 3600 * 1000, "停放不应超过一天");
    }
}
