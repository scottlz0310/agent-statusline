/// 残り秒数を "now", "45m", "4h50m", "6d23h" 形式にフォーマット
pub fn format_reset_seconds(seconds: i64) -> String {
    if seconds <= 0 {
        return "now".to_string();
    }
    let mins = seconds / 60;
    if mins < 60 {
        return format!("{mins}m");
    }
    let hrs = mins / 60;
    let mins_rem = mins % 60;
    if hrs < 24 {
        return format!("{hrs}h{mins_rem}m");
    }
    let days = hrs / 24;
    let hrs_rem = hrs % 24;
    format!("{days}d{hrs_rem}h")
}
