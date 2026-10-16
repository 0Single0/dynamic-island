//! 时间工具：纪元毫秒、公历↔日序、**本地**时间。
//!
//! 桌面端在 Windows 上直接问 `GetLocalTime`。服务器版跑在 Linux 容器里，
//! 没有等价的零依赖接口，所以改成：读 `TZ` 环境变量解析 UTC 偏移。
//!
//! 为什么不能简单用 UTC 近似：每日动作（签到 / 猫猫旅行 / 每日活跃对话）按
//! **本地日期**判断"今天做没做"，用量页的"今日"也按本地零点切桶。
//! 容器里 UTC 与北京时间差 8 小时，直接用 UTC 会让跨零点前后的行为错位。
//!
//! 支持的 `TZ` 形态（覆盖容器部署的常见写法）：
//! - `Asia/Shanghai`、`UTC`、`Etc/GMT-8` 这类区域名 → 查内置常用表；
//! - `UTC+8` / `GMT-5` / `+08:00` 这类显式偏移 → 直接解析；
//! - 未设或无法识别 → 回落 UTC（启动时由 `main` 打印提示）。

use std::time::{SystemTime, UNIX_EPOCH};

/// 当前纪元毫秒（UTC）。全仓统一用它，避免各处自己 `SystemTime::now()` 后比较口径不同。
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

/// 日序 → (年, 月, 日)：`days_from_civil` 的逆运算。
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
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

/// 当前 `TZ` 对应的 UTC 偏移（秒）。无法识别时回落 0（UTC）。
pub fn local_utc_offset_secs() -> i64 {
    match std::env::var("TZ") {
        Ok(tz) => parse_tz_offset(&tz).unwrap_or(0),
        Err(_) => 0,
    }
}

/// 解析 `TZ` 为 UTC 偏移秒。识别不了返回 `None`。
pub fn parse_tz_offset(tz: &str) -> Option<i64> {
    let tz = tz.trim();
    if tz.is_empty() {
        return None;
    }
    // ① 显式偏移：UTC+8 / GMT-5 / +08:00 / -0330
    if let Some(offset) = parse_explicit_offset(tz) {
        return Some(offset);
    }
    // ② 区域名：查常用表（容器部署基本就这几个）。
    let canonical = tz.replace(' ', "_").to_ascii_lowercase();
    if let Some(offset) = lookup_zone(&canonical) {
        return Some(offset);
    }
    // `Asia/Shanghai` 这类：用最后一段再试一次（`shanghai`）。
    let tail = canonical.rsplit('/').next().unwrap_or(&canonical).to_string();
    lookup_zone(&tail)
}

fn lookup_zone(name: &str) -> Option<i64> {
    COMMON_ZONES
        .iter()
        .find(|(zone, _)| *zone == name)
        .map(|(_, offset)| *offset)
}

/// `UTC+8`、`GMT-5`、`+08:00`、`-0330` 这类显式偏移。
fn parse_explicit_offset(tz: &str) -> Option<i64> {
    let upper = tz.to_ascii_uppercase();
    let rest = if let Some(rest) = upper.strip_prefix("UTC") {
        rest
    } else if let Some(rest) = upper.strip_prefix("GMT") {
        rest
    } else if upper.starts_with('+') || upper.starts_with('-') {
        upper.as_str()
    } else {
        return None;
    };
    let rest = rest.trim();
    if rest.is_empty() {
        // 纯 `UTC` / `GMT` = 0。
        return Some(0);
    }
    let (sign, _) = match rest.chars().next()? {
        '+' => (1i64, ()),
        '-' => (-1i64, ()),
        _ => return None,
    };
    let has_colon = rest[1..].contains(':');
    let digits: String = rest[1..].chars().filter(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let (hours, minutes) = if has_colon {
        // `+08:00` / `-03:30`
        let raw: Vec<&str> = rest[1..].split(':').collect();
        let hours: i64 = raw.first().unwrap_or(&"0").trim().parse().ok()?;
        let minutes: i64 = raw.get(1).unwrap_or(&"0").trim().parse().ok()?;
        (hours, minutes)
    } else {
        match digits.len() {
            // `+8` = 8 小时
            1 | 2 => (digits.parse().ok()?, 0),
            // `+0800` / `-0330`
            _ => {
                let (h, m) = digits.split_at(digits.len() - 2);
                (h.parse().ok()?, m.parse().ok()?)
            }
        }
    };
    Some(sign * (hours * 3600 + minutes * 60))
}

/// 本地日期与小时 `(年, 月, 日, 时)` —— 每日动作按**本地**日期判断"今天做没做"。
pub fn local_date_hour() -> (i32, u32, u32, u32) {
    let offset = local_utc_offset_secs();
    let local_secs = (now_ms() / 1000) as i64 + offset;
    let days = local_secs.div_euclid(86_400);
    let secs_of_day = local_secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    (year as i32, month, day, (secs_of_day / 3600) as u32)
}

/// 本地日期的零点（纪元毫秒）——用量页"今日"分桶用。
pub fn local_today_start_ms() -> u64 {
    let offset = local_utc_offset_secs();
    let local_secs = (now_ms() / 1000) as i64 + offset;
    let days = local_secs.div_euclid(86_400);
    (days * 86_400 - offset).max(0) as u64 * 1000
}

/// 下一个「本机时区 04:00」的纪元毫秒（余额停放用：避开北京时间凌晨的结算点）。
pub fn next_local_4am_ms(now_ms: u64) -> u64 {
    let offset = local_utc_offset_secs();
    let local_secs = (now_ms / 1000) as i64 + offset;
    let days = local_secs.div_euclid(86_400);
    let now_secs = (now_ms / 1000) as i64;
    // 本地当天 04:00 对应的 UTC 秒。
    let mut target = days * 86_400 + 4 * 3600 - offset;
    if target <= now_secs {
        target += 86_400;
    }
    // 时钟回拨或时区跳变的极端情况下，至少停放一分钟，避免立刻重试打成死循环。
    (target.max(now_secs + 60)) as u64 * 1000
}

/// 常用时区 → UTC 偏移秒。只覆盖部署里常见的区域名。
const COMMON_ZONES: &[(&str, i64)] = &[
    ("utc", 0),
    ("gmt", 0),
    ("asia/shanghai", 8 * 3600),
    ("shanghai", 8 * 3600),
    ("asia/chongqing", 8 * 3600),
    ("asia/harbin", 8 * 3600),
    ("asia/urumqi", 8 * 3600),
    ("prc", 8 * 3600),
    ("asia/hong_kong", 8 * 3600),
    ("hongkong", 8 * 3600),
    ("asia/taipei", 8 * 3600),
    ("asia/singapore", 8 * 3600),
    ("asia/tokyo", 9 * 3600),
    ("japan", 9 * 3600),
    ("asia/seoul", 9 * 3600),
    ("asia/kolkata", 5 * 3600 + 1800),
    ("asia/dubai", 4 * 3600),
    ("europe/london", 0),
    ("europe/paris", 3600),
    ("europe/berlin", 3600),
    ("europe/moscow", 3 * 3600),
    ("america/new_york", -5 * 3600),
    ("america/los_angeles", -8 * 3600),
    ("america/chicago", -6 * 3600),
    ("australia/sydney", 10 * 3600),
];

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
    fn civil_from_days_is_the_inverse() {
        for days in [-10_000i64, -1, 0, 1, 19_000, 40_000] {
            let (year, month, day) = civil_from_days(days);
            assert_eq!(
                days_from_civil(year, i64::from(month), i64::from(day)),
                days,
                "days={days}"
            );
        }
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
    fn explicit_offsets_parse() {
        assert_eq!(parse_tz_offset("UTC+8"), Some(8 * 3600));
        assert_eq!(parse_tz_offset("GMT-5"), Some(-5 * 3600));
        assert_eq!(parse_tz_offset("+08:00"), Some(8 * 3600));
        assert_eq!(parse_tz_offset("-0330"), Some(-(3 * 3600 + 1800)));
        assert_eq!(parse_tz_offset("UTC"), Some(0));
    }

    #[test]
    fn region_names_parse() {
        assert_eq!(parse_tz_offset("Asia/Shanghai"), Some(8 * 3600));
        assert_eq!(parse_tz_offset("America/New_York"), Some(-5 * 3600));
        assert_eq!(parse_tz_offset("Etc/Unknown"), None);
    }

    #[test]
    fn balance_parking_is_always_in_the_future() {
        let now = now_ms();
        let target = next_local_4am_ms(now);
        assert!(target > now, "停放时刻必须在将来：{target} <= {now}");
        assert!(target <= now + 25 * 3600 * 1000, "停放不应超过一天");
    }

    #[test]
    fn today_start_is_within_the_last_day() {
        let start = local_today_start_ms();
        let now = now_ms();
        assert!(start <= now, "今日零点不能晚于现在");
        assert!(start + 25 * 3600 * 1000 > now, "今日零点不该早于一天前");
    }
}
