use chrono::{Datelike, Duration, Local, NaiveDateTime, Timelike};

// ── Schedule ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum CronSchedule {
    Standard {
        minute: String,
        hour: String,
        day: String,
        month: String,
        weekday: String,
    },
    Special(String),
}

impl CronSchedule {
    pub fn display(&self) -> String {
        match self {
            CronSchedule::Standard {
                minute,
                hour,
                day,
                month,
                weekday,
            } => format!("{} {} {} {} {}", minute, hour, day, month, weekday),
            CronSchedule::Special(s) => s.clone(),
        }
    }

    pub fn describe(&self, use_24h: bool) -> String {
        match self {
            CronSchedule::Special(s) => describe_special(s),
            CronSchedule::Standard {
                minute,
                hour,
                day,
                month,
                weekday,
            } => describe_standard(minute, hour, day, month, weekday, use_24h),
        }
    }

    pub fn next_run(&self, use_24h: bool) -> Option<String> {
        let now = Local::now().naive_local();
        let next = self.next_after(now)?;
        Some(format_next(now, next, use_24h))
    }

    /// Compute up to `n` future run times starting from now.
    pub fn next_n_runs(&self, n: usize, use_24h: bool) -> Vec<(NaiveDateTime, String)> {
        let now = Local::now().naive_local();
        let mut results = Vec::new();
        let mut from = now;
        for _ in 0..n {
            match self.next_after(from) {
                Some(t) => {
                    results.push((t, format_abs(t, use_24h)));
                    from = t;
                }
                None => break,
            }
        }
        results
    }

    /// Firings per hour in a typical 24-hour period (ignores dom/month/dow constraints).
    pub fn firings_per_hour(&self) -> [u8; 24] {
        let mut counts = [0u8; 24];
        match self {
            CronSchedule::Special(s) => match s.to_lowercase().as_str() {
                "@reboot" => {}
                "@hourly" => counts.iter_mut().for_each(|c| *c = 1),
                "@daily" | "@midnight" => {
                    counts[0] = 1;
                }
                "@weekly" => {
                    counts[0] = 1;
                }
                "@monthly" => {
                    counts[0] = 1;
                }
                "@yearly" | "@annually" => {
                    counts[0] = 1;
                }
                _ => {}
            },
            CronSchedule::Standard { minute, hour, .. } => {
                let mins = expand(minute, 0, 59, &[]).unwrap_or_default();
                let hrs = expand(hour, 0, 23, &[]).unwrap_or_default();
                for h in hrs {
                    counts[h as usize] = mins.len() as u8;
                }
            }
        }
        counts
    }

    /// True when every field parses (and, for @specials, the keyword is known).
    pub fn is_valid(&self) -> bool {
        match self {
            CronSchedule::Special(s) => SPECIALS.iter().any(|e| e.keyword.eq_ignore_ascii_case(s)),
            CronSchedule::Standard {
                minute,
                hour,
                day,
                month,
                weekday,
            } => {
                field_valid(minute, 0, 59, &[])
                    && field_valid(hour, 0, 23, &[])
                    && field_valid(day, 1, 31, &[])
                    && field_valid(month, 1, 12, &MONTH_ABBREV)
                    && field_valid(weekday, 0, 6, &WD_ABBREV)
            }
        }
    }

    fn next_after(&self, from: NaiveDateTime) -> Option<NaiveDateTime> {
        match self {
            CronSchedule::Special(s) => next_special(s, from),
            CronSchedule::Standard {
                minute,
                hour,
                day,
                month,
                weekday,
            } => next_standard(minute, hour, day, month, weekday, from),
        }
    }
}

// ── Describe: @special ────────────────────────────────────────────────────────

pub fn describe_special(s: &str) -> String {
    match s.to_lowercase().as_str() {
        "@reboot" => "At system startup".into(),
        "@yearly" | "@annually" => "Yearly on Jan 1 at 00:00".into(),
        "@monthly" => "Monthly on the 1st at 00:00".into(),
        "@weekly" => "Weekly on Sundays at 00:00".into(),
        "@daily" | "@midnight" => "Daily at 00:00".into(),
        "@hourly" => "Every hour at :00".into(),
        other => format!("Special: {}", other),
    }
}

// ── Describe: standard 5-field (the main brains) ──────────────────────────────

fn describe_standard(
    minute: &str,
    hour: &str,
    day: &str,
    month: &str,
    weekday: &str,
    use_24h: bool,
) -> String {
    if [minute, hour, day, month, weekday]
        .iter()
        .all(|f| *f == "*")
    {
        return "Every minute".into();
    }

    let mins = match expand(minute, 0, 59, &[]) {
        Some(v) => v,
        None => return "Invalid schedule".into(),
    };
    let hrs = match expand(hour, 0, 23, &[]) {
        Some(v) => v,
        None => return "Invalid schedule".into(),
    };

    // ── Pure-frequency early returns (no day/month/weekday constraints) ───────

    if day == "*" && month == "*" && weekday == "*" && hour == "*" {
        // */N * * * *  or  * * * * *
        if let Some(n) = step_n(minute) {
            return if n <= 1 {
                "Every minute".into()
            } else {
                format!("Every {} minutes", n)
            };
        }
        // M * * * * — specific minute, every hour
        if mins == [0] {
            return "Every hour".into();
        }
        let strs: Vec<String> = mins.iter().map(|&m| format!(":{:02}", m)).collect();
        return format!("At {} past every hour", join_and(&strs));
    }

    // ── Time component ────────────────────────────────────────────────────────

    let time = build_time_component(minute, hour, &mins, &hrs, use_24h);

    // ── Month component ───────────────────────────────────────────────────────

    let mon_desc: Option<String> = if month == "*" {
        None
    } else {
        describe_month_expr(month)
    };

    // ── Assemble ──────────────────────────────────────────────────────────────

    if weekday != "*" && day == "*" {
        let wd = describe_weekdays(weekday);
        return match (&time, &mon_desc) {
            (Some(t), Some(m)) => format!("{} in {} at {}", wd, m, t),
            (Some(t), None) => format!("{} at {}", wd, t),
            (None, Some(m)) => format!("{} in {}", wd, m),
            (None, None) => wd,
        };
    }

    if day != "*" && weekday == "*" {
        let dom = describe_dom(day);
        return match (&time, &mon_desc) {
            (Some(t), Some(m)) => format!("{}, on {} in {}", capitalize_first(t), dom, m),
            (Some(t), None) => format!("Monthly on {} at {}", dom, t),
            (None, Some(m)) => format!("{} on {}", m, dom),
            (None, None) => format!("Monthly on {}", dom),
        };
    }

    if day != "*" && weekday != "*" {
        // Vixie cron: when both day-of-month and weekday are restricted,
        // the job fires when EITHER matches.
        let dom = describe_dom(day);
        let wd = describe_weekday_on(weekday);
        return match (&time, &mon_desc) {
            (Some(t), Some(m)) => format!(
                "{} on {} or on {} in {}",
                describe_when_prefix(t),
                dom,
                wd,
                m
            ),
            (Some(t), None) => format!("{} on {} or on {}", describe_when_prefix(t), dom, wd),
            (None, Some(m)) => format!("On {} or on {} in {}", dom, wd, m),
            (None, None) => format!("On {} or on {}", dom, wd),
        };
    }

    match (&time, &mon_desc) {
        (Some(t), Some(m)) => format!("Daily in {} at {}", m, t),
        (Some(t), None) => format!("Daily at {}", t),
        (None, Some(m)) => format!("Every day in {}", m),
        (None, None) => "Every day".into(),
    }
}

/// Detect plain step pattern: returns N for */N, 1 for *, None otherwise.
fn step_n(s: &str) -> Option<u32> {
    if s == "*" {
        return Some(1);
    }
    s.strip_prefix("*/")?.parse().ok()
}

fn capitalize_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

// ── Build time component ──────────────────────────────────────────────────────

fn build_time_component(
    minute: &str,
    hour: &str,
    mins: &[u8],
    hrs: &[u8],
    use_24h: bool,
) -> Option<String> {
    let all_hrs = hour == "*";

    // Detect step pattern: */N  or  *
    let step: Option<u8> = if minute == "*" {
        Some(1)
    } else {
        minute.strip_prefix("*/").and_then(|s| s.parse().ok())
    };

    if let Some(n) = step {
        if all_hrs {
            // */N * ... → "Every N minutes" or "Every minute"
            return if n <= 1 {
                None
            }
            // handled at caller as "Every minute"
            else {
                Some(format!("every {} minutes", n))
            };
        }
        // */N H,H,H ... → windows per hour
        let last_min: u8 = (59 / n) * n;
        let windows: Vec<String> = hrs
            .iter()
            .map(|&h| {
                format!(
                    "{} to {}",
                    fmt_time(h, 0, use_24h),
                    fmt_time(h, last_min, use_24h)
                )
            })
            .collect();
        return Some(format!(
            "every {} minute{} from {}",
            n,
            if n == 1 { "" } else { "s" },
            join_and(&windows)
        ));
    }

    // Not a step — specific minutes
    if all_hrs {
        // specific minute, every hour
        if mins == [0] {
            return Some("every hour".into());
        }
        let strs: Vec<String> = mins.iter().map(|&m| format!(":{:02}", m)).collect();
        return Some(format!("at {} past every hour", join_and(&strs)));
    }

    // Specific minutes + specific hours → enumerate all fire times
    let times: Vec<String> = hrs
        .iter()
        .flat_map(|&h| mins.iter().map(move |&m| fmt_time(h, m, use_24h)))
        .collect();

    if times.len() <= 8 {
        Some(join_and(&times))
    } else {
        Some(format!("{} times per day", times.len()))
    }
}

// ── Month description ─────────────────────────────────────────────────────────

fn describe_months(mv: &[u8]) -> String {
    if mv.len() == 1 {
        return month_name(mv[0]);
    }
    // Contiguous range?
    if mv.windows(2).all(|w| w[1] == w[0] + 1) {
        return format!(
            "{} through {}",
            month_name(mv[0]),
            month_name(*mv.last().unwrap())
        );
    }
    let names: Vec<String> = mv.iter().map(|&m| month_name(m)).collect();
    join_and(&names)
}

fn describe_month_expr(s: &str) -> Option<String> {
    if let Some(n) = step_n(s) {
        return if n <= 1 {
            Some("every month".into())
        } else {
            Some(format!("every {} month", ordinal(n)))
        };
    }
    expand(s, 1, 12, &MONTH_ABBREV).map(|mv| describe_months(&mv))
}

// ── Weekday description ───────────────────────────────────────────────────────

const WD_PLURAL: [&str; 7] = [
    "Sundays",
    "Mondays",
    "Tuesdays",
    "Wednesdays",
    "Thursdays",
    "Fridays",
    "Saturdays",
];
const WD_SINGULAR: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const WD_ABBREV: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const MONTH_ABBREV: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

fn describe_weekdays(s: &str) -> String {
    // Range
    if let Some((a, b)) = s.split_once('-')
        && let (Some(si), Some(ei)) = (parse_wd(a), parse_wd(b))
    {
        if si == 1 && ei == 5 {
            return "Weekdays".into();
        }
        if si == 0 && ei == 6 {
            return "Every day".into();
        }
        return format!("{} through {}", WD_PLURAL[si], WD_PLURAL[ei % 7]);
    }
    // Comma list
    if s.contains(',') {
        let names: Vec<&str> = s
            .split(',')
            .filter_map(|p| parse_wd(p.trim()))
            .map(|i| WD_PLURAL[i])
            .collect();
        if !names.is_empty() {
            return join_and_strs(&names);
        }
    }
    // Single
    if let Some(i) = parse_wd(s) {
        return WD_PLURAL[i].into();
    }
    s.into()
}

fn describe_weekday_on(s: &str) -> String {
    if let Some(i) = parse_wd(s) {
        return WD_SINGULAR[i].into();
    }
    if s.contains(',') {
        let names: Vec<&str> = s
            .split(',')
            .filter_map(|p| parse_wd(p.trim()))
            .map(|i| WD_SINGULAR[i])
            .collect();
        if !names.is_empty() {
            return join_and_strs(&names);
        }
    }
    describe_weekdays(s)
}

fn describe_when_prefix(t: &str) -> String {
    if t.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("At {}", t)
    } else {
        capitalize_first(t)
    }
}

fn parse_wd(s: &str) -> Option<usize> {
    let mut n: usize = s.parse().ok().or_else(|| {
        WD_ABBREV
            .iter()
            .position(|&a| a == s.to_lowercase().as_str())
    })?;
    if n == 7 {
        n = 0;
    }
    if n < 7 { Some(n) } else { None }
}

// ── DOM description ───────────────────────────────────────────────────────────

fn describe_dom(s: &str) -> String {
    if let Ok(n) = s.parse::<u32>() {
        return format!("the {}", ordinal(n));
    }
    if let Some((a, b)) = s.split_once('-')
        && let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>())
    {
        return format!("days {} to {}", a, b);
    }
    if s.contains(',')
        && let Some(nums) = s
            .split(',')
            .map(|p| p.trim().parse::<u32>().ok())
            .collect::<Option<Vec<_>>>()
    {
        let strs: Vec<String> = nums
            .iter()
            .map(|&n| format!("the {}", ordinal(n)))
            .collect();
        return join_and(&strs);
    }
    s.into()
}

// ── Utility ───────────────────────────────────────────────────────────────────

pub fn fmt_time(h: u8, m: u8, use_24h: bool) -> String {
    if use_24h {
        format!("{:02}:{:02}", h, m)
    } else {
        let period = if h < 12 { "AM" } else { "PM" };
        let dh: u8 = match h {
            0 => 12,
            13..=23 => h - 12,
            _ => h,
        };
        if m == 0 {
            format!("{} {}", dh, period)
        } else {
            format!("{}:{:02} {}", dh, m, period)
        }
    }
}

fn join_and(parts: &[String]) -> String {
    match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        2 => format!("{} and {}", parts[0], parts[1]),
        n => format!("{}, and {}", parts[..n - 1].join(", "), parts[n - 1]),
    }
}

fn join_and_strs(parts: &[&str]) -> String {
    match parts.len() {
        0 => String::new(),
        1 => parts[0].to_string(),
        2 => format!("{} and {}", parts[0], parts[1]),
        n => format!("{}, and {}", parts[..n - 1].join(", "), parts[n - 1]),
    }
}

fn month_name(n: u8) -> String {
    const N: [&str; 13] = [
        "",
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    N.get(n as usize).copied().unwrap_or("?").to_string()
}

fn ordinal(n: u32) -> String {
    let suf = match n % 100 {
        11..=13 => "th",
        _ => match n % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        },
    };
    format!("{}{}", n, suf)
}

fn format_abs(t: NaiveDateTime, use_24h: bool) -> String {
    let h = t.hour() as u8;
    let m = t.minute() as u8;
    let time_s = fmt_time(h, m, use_24h);
    let day_s = t.format("%a %b %-d").to_string();
    format!("{} {}", day_s, time_s)
}

fn format_next(now: NaiveDateTime, next: NaiveDateTime, use_24h: bool) -> String {
    let diff = next - now;
    let mins = diff.num_minutes();
    let hours = diff.num_hours();
    let days = diff.num_days();

    let rel = if mins < 60 {
        format!("in {}m", mins)
    } else if hours < 24 {
        let m = mins - hours * 60;
        if m == 0 {
            format!("in {}h", hours)
        } else {
            format!("in {}h {}m", hours, m)
        }
    } else if days < 7 {
        format!("in {} day{}", days, if days == 1 { "" } else { "s" })
    } else {
        next.format("%b %-d").to_string()
    };

    let h = next.hour() as u8;
    let m = next.minute() as u8;
    let time_s = fmt_time(h, m, use_24h);
    if now.date() == next.date() {
        format!("{} ({})", time_s, rel)
    } else if days < 7 {
        let abbr = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
            [next.weekday().num_days_from_sunday() as usize];
        format!("{} {} ({})", abbr, time_s, rel)
    } else {
        format!("{} {} ({})", next.format("%b %-d"), time_s, rel)
    }
}

// ── Next-run: @special ────────────────────────────────────────────────────────

fn next_special(s: &str, now: NaiveDateTime) -> Option<NaiveDateTime> {
    match s.to_lowercase().as_str() {
        "@reboot" => None,
        "@yearly" | "@annually" => next_standard("0", "0", "1", "1", "*", now),
        "@monthly" => next_standard("0", "0", "1", "*", "*", now),
        "@weekly" => next_standard("0", "0", "*", "*", "0", now),
        "@daily" | "@midnight" => next_standard("0", "0", "*", "*", "*", now),
        "@hourly" => {
            let s = now + Duration::seconds(60);
            let t = s.with_minute(0)?.with_second(0)?;
            if t > s {
                Some(t)
            } else {
                Some(t + Duration::hours(1))
            }
        }
        _ => None,
    }
}

// ── Next-run: 5-field ─────────────────────────────────────────────────────────

pub(crate) fn next_standard(
    min_s: &str,
    hr_s: &str,
    dom_s: &str,
    mon_s: &str,
    dow_s: &str,
    now: NaiveDateTime,
) -> Option<NaiveDateTime> {
    let minutes = expand(min_s, 0, 59, &[])?;
    let hours = expand(hr_s, 0, 23, &[])?;
    let doms = expand(dom_s, 1, 31, &[])?;
    let months = expand(mon_s, 1, 12, &MONTH_ABBREV)?;
    let dows = expand(dow_s, 0, 6, &WD_ABBREV)?;
    let dom_star = dom_s == "*";
    let dow_star = dow_s == "*";

    let mut t = (now + Duration::seconds(60)).with_second(0).unwrap_or(now);
    let limit = now + Duration::days(366);

    while t < limit {
        let mon = t.month() as u8;
        if !months.contains(&mon) {
            t = advance_to_month(t, &months)?;
            continue;
        }

        let dom = t.day() as u8;
        let dow = t.weekday().num_days_from_sunday() as u8;
        // Vixie cron: when both dom and dow are restricted, either match fires.
        let day_ok = if dom_star && dow_star {
            true
        } else if dom_star {
            dows.contains(&dow)
        } else if dow_star {
            doms.contains(&dom)
        } else {
            doms.contains(&dom) || dows.contains(&dow)
        };
        if !day_ok {
            t = (t + Duration::days(1))
                .with_hour(0)?
                .with_minute(0)?
                .with_second(0)?;
            continue;
        }

        let hr = t.hour() as u8;
        if !hours.contains(&hr) {
            if let Some(nh) = hours.iter().find(|&&h| h > hr) {
                t = t.with_hour(*nh as u32)?.with_minute(0)?.with_second(0)?;
            } else {
                t = (t + Duration::days(1))
                    .with_hour(0)?
                    .with_minute(0)?
                    .with_second(0)?;
            }
            continue;
        }

        let mn = t.minute() as u8;
        if let Some(&nm) = minutes.iter().find(|&&m| m >= mn) {
            return t.with_minute(nm as u32)?.with_second(0);
        } else if let Some(nh) = hours.iter().find(|&&h| h > hr) {
            t = t.with_hour(*nh as u32)?.with_minute(0)?.with_second(0)?;
        } else {
            t = (t + Duration::days(1))
                .with_hour(0)?
                .with_minute(0)?
                .with_second(0)?;
        }
    }
    None
}

fn advance_to_month(t: NaiveDateTime, months: &[u8]) -> Option<NaiveDateTime> {
    // Build the date from scratch to avoid invalid-day errors when e.g. Jan 31 → Feb.
    let cur = t.month() as u8;
    let (year, next_mon) = if let Some(&m) = months.iter().find(|&&m| m > cur) {
        (t.year(), m as u32)
    } else {
        (t.year() + 1, months[0] as u32)
    };
    chrono::NaiveDate::from_ymd_opt(year, next_mon, 1)?.and_hms_opt(0, 0, 0)
}

// ── Field expansion ───────────────────────────────────────────────────────────

/// Parse a single field atom: a number or a name from `names` (e.g. "mon", "jan").
/// Numeric 7 is aliased to 0 (Sunday) when the field range is 0–6.
fn parse_atom(s: &str, lo: u8, hi: u8, names: &[&str]) -> Option<u8> {
    if let Ok(n) = s.parse::<u8>() {
        let n = if n == 7 && hi == 6 { 0 } else { n };
        return (lo..=hi).contains(&n).then_some(n);
    }
    let idx = names.iter().position(|&a| a.eq_ignore_ascii_case(s))?;
    Some(lo + idx as u8)
}

pub fn expand(expr: &str, lo: u8, hi: u8, names: &[&str]) -> Option<Vec<u8>> {
    let mut set: Vec<u8> = Vec::new();
    for part in expr.split(',') {
        let part = part.trim();
        if part == "*" {
            set.extend(lo..=hi);
            continue;
        }
        if let Some(step_s) = part.strip_prefix("*/") {
            let step: u8 = step_s.parse().ok()?;
            if step == 0 {
                return None;
            }
            let mut v = lo;
            while v <= hi {
                set.push(v);
                v = v.saturating_add(step);
            }
            continue;
        }
        if let Some(dash) = part.find('-') {
            let a_s = &part[..dash];
            let rest = &part[dash + 1..];
            let (b_s, step) = if let Some(sl) = rest.find('/') {
                (&rest[..sl], rest[sl + 1..].parse::<u8>().ok()?)
            } else {
                (rest, 1u8)
            };
            let a = parse_atom(a_s, lo, hi, names)?;
            // Keep "5-7" usable as a weekday range end (7 = Saturday cap).
            let b = if b_s == "7" && hi == 6 {
                6
            } else {
                parse_atom(b_s, lo, hi, names)?
            };
            if a > b {
                return None;
            }
            let mut v = a;
            while v <= b {
                set.push(v);
                v = v.saturating_add(step);
            }
            continue;
        }
        set.push(parse_atom(part, lo, hi, names)?);
    }
    set.sort_unstable();
    set.dedup();
    if set.is_empty() { None } else { Some(set) }
}

fn field_valid(s: &str, lo: u8, hi: u8, names: &[&str]) -> bool {
    expand(s, lo, hi, names).is_some()
}

// ── Data model ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CronEntry {
    pub enabled: bool,
    pub schedule: CronSchedule,
    pub command: String,
}

impl CronEntry {
    pub fn default_new() -> Self {
        CronEntry {
            enabled: true,
            schedule: CronSchedule::Standard {
                minute: "0".into(),
                hour: "9".into(),
                day: "*".into(),
                month: "*".into(),
                weekday: "*".into(),
            },
            command: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum CrontabLine {
    Blank,
    Comment(String),
    Variable { key: String, value: String },
    Entry(CronEntry),
}

// ── Parsing ───────────────────────────────────────────────────────────────────

pub fn parse_crontab(content: &str) -> Vec<CrontabLine> {
    content.lines().map(parse_line).collect()
}

fn parse_line(raw: &str) -> CrontabLine {
    if raw.trim().is_empty() {
        return CrontabLine::Blank;
    }
    if raw.trim_start().starts_with('#') {
        let inner = raw.trim_start().trim_start_matches('#').trim();
        if let Some(e) = try_parse_entry(inner) {
            return CrontabLine::Entry(CronEntry {
                enabled: false,
                ..e
            });
        }
        return CrontabLine::Comment(raw.into());
    }
    if let Some(eq) = raw.find('=') {
        let key = &raw[..eq];
        if !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return CrontabLine::Variable {
                key: key.into(),
                value: raw[eq + 1..].into(),
            };
        }
    }
    if let Some(e) = try_parse_entry(raw) {
        return CrontabLine::Entry(e);
    }
    CrontabLine::Comment(raw.into())
}

fn try_parse_entry(s: &str) -> Option<CronEntry> {
    let s = s.trim();
    if s.starts_with('@') {
        let kw = s.split_whitespace().next()?;
        let cmd = s[kw.len()..].trim();
        if cmd.is_empty() {
            return None;
        }
        return Some(CronEntry {
            enabled: true,
            schedule: CronSchedule::Special(kw.into()),
            command: cmd.into(),
        });
    }
    let mut p = s.split_whitespace();
    let min = p.next()?.to_string();
    let hr = p.next()?.to_string();
    let dom = p.next()?.to_string();
    let mon = p.next()?.to_string();
    let dow = p.next()?.to_string();
    let cmd: String = p.collect::<Vec<_>>().join(" ");
    if cmd.is_empty() {
        return None;
    }
    // Strict field validation
    if !field_valid(&min, 0, 59, &[]) {
        return None;
    }
    if !field_valid(&hr, 0, 23, &[]) {
        return None;
    }
    if !field_valid(&dom, 1, 31, &[]) {
        return None;
    }
    if !field_valid(&mon, 1, 12, &MONTH_ABBREV) {
        return None;
    }
    if !field_valid(&dow, 0, 6, &WD_ABBREV) {
        return None;
    }
    Some(CronEntry {
        enabled: true,
        schedule: CronSchedule::Standard {
            minute: min,
            hour: hr,
            day: dom,
            month: mon,
            weekday: dow,
        },
        command: cmd,
    })
}

// ── Serialization ─────────────────────────────────────────────────────────────

pub fn serialize_crontab(lines: &[CrontabLine]) -> String {
    let mut out = String::new();
    for line in lines {
        match line {
            CrontabLine::Blank => out.push('\n'),
            CrontabLine::Comment(s) => {
                out.push_str(s);
                out.push('\n');
            }
            CrontabLine::Variable { key, value } => {
                out.push_str(&format!("{}={}\n", key, value));
            }
            CrontabLine::Entry(e) => {
                if !e.enabled {
                    out.push_str("# ");
                }
                out.push_str(&e.schedule.display());
                out.push(' ');
                out.push_str(&e.command);
                out.push('\n');
            }
        }
    }
    out
}

// ── @special catalogue ────────────────────────────────────────────────────────

pub struct SpecialEntry {
    pub keyword: &'static str,
    #[allow(dead_code)]
    pub desc: &'static str,
}

pub const SPECIALS: &[SpecialEntry] = &[
    SpecialEntry {
        keyword: "@reboot",
        desc: "At system startup",
    },
    SpecialEntry {
        keyword: "@yearly",
        desc: "Once a year — Jan 1 at midnight",
    },
    SpecialEntry {
        keyword: "@annually",
        desc: "Same as @yearly",
    },
    SpecialEntry {
        keyword: "@monthly",
        desc: "1st of month at midnight",
    },
    SpecialEntry {
        keyword: "@weekly",
        desc: "Sunday at midnight",
    },
    SpecialEntry {
        keyword: "@daily",
        desc: "Daily at midnight",
    },
    SpecialEntry {
        keyword: "@midnight",
        desc: "Same as @daily",
    },
    SpecialEntry {
        keyword: "@hourly",
        desc: "Every hour at :00",
    },
];

// ── Field help strings ────────────────────────────────────────────────────────

pub const FIELD_HELP: [(&str, &str, &str); 5] = [
    ("Minute", "0–59", "*/5  0,15,30,45  10-20  0"),
    ("Hour", "0–23", "*/2  9,17  8-18   0"),
    ("Day", "1–31", "*/5  1,15  1-7"),
    ("Month", "1–12", "*/3  2-4   JAN,JUN,DEC"),
    ("Weekday", "0–7 (0/7=Sun)", "1-5  0,6  MON-FRI"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn sched(s: &str) -> CronSchedule {
        let mut p = s.split_whitespace();
        CronSchedule::Standard {
            minute: p.next().unwrap().into(),
            hour: p.next().unwrap().into(),
            day: p.next().unwrap().into(),
            month: p.next().unwrap().into(),
            weekday: p.next().unwrap().into(),
        }
    }

    fn next_after(spec: &str, from_str: &str) -> String {
        let from = NaiveDateTime::parse_from_str(from_str, "%Y-%m-%d %H:%M").unwrap();
        match sched(spec) {
            CronSchedule::Standard {
                minute,
                hour,
                day,
                month,
                weekday,
            } => next_standard(&minute, &hour, &day, &month, &weekday, from)
                .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "None".into()),
            _ => "special".into(),
        }
    }

    #[test]
    fn next_runs() {
        assert_eq!(
            next_after("*/15 * * * *", "2025-04-22 09:08"),
            "2025-04-22 09:15"
        );
        assert_eq!(
            next_after("*/15 * * * *", "2025-04-22 09:15"),
            "2025-04-22 09:30"
        );
        assert_eq!(
            next_after("0 * * * *", "2025-04-22 09:00"),
            "2025-04-22 10:00"
        );
        assert_eq!(
            next_after("0 2 * * *", "2025-04-22 09:00"),
            "2025-04-23 02:00"
        );
        assert_eq!(
            next_after("30 2 * * 5", "2025-04-22 09:00"),
            "2025-04-25 02:30"
        );
        assert_eq!(
            next_after("30 3 1 * *", "2025-04-22 09:00"),
            "2025-05-01 03:30"
        );
        assert_eq!(
            next_after("0 4,5 * * *", "2025-04-22 03:30"),
            "2025-04-22 04:00"
        );
        assert_eq!(
            next_after("0 4,5 * * *", "2025-04-22 04:30"),
            "2025-04-22 05:00"
        );
        assert_eq!(
            next_after("0 4,5 * * *", "2025-04-22 05:30"),
            "2025-04-23 04:00"
        );
        assert_eq!(
            next_after("0 4 * * 0,3", "2025-04-22 09:00"),
            "2025-04-23 04:00"
        );
        assert_eq!(
            next_after("*/5 9,12 1 2-4 *", "2025-01-31 00:00"),
            "2025-02-01 09:00"
        );
        assert_eq!(
            next_after("*/5 9,12 1 2-4 *", "2025-02-01 09:03"),
            "2025-02-01 09:05"
        );
        assert_eq!(
            next_after("*/5 9,12 1 2-4 *", "2025-02-01 09:55"),
            "2025-02-01 12:00"
        );
    }

    #[test]
    fn dom_dow_both_restricted_fires_on_either() {
        // Vixie cron OR semantics: dom=1 or Monday, whichever comes first.
        // From Tue 2025-04-22, next Monday (Apr 28) precedes May 1.
        assert_eq!(
            next_after("0 4 1 * 1", "2025-04-22 09:00"),
            "2025-04-28 04:00"
        );
        // From Apr 29 (after that Monday? no — Apr 28 passed), May 1 (Thu) precedes next Monday May 5.
        assert_eq!(
            next_after("0 4 1 * 1", "2025-04-29 09:00"),
            "2025-05-01 04:00"
        );
    }

    #[test]
    fn next_run_accepts_weekday_and_month_names() {
        assert_eq!(
            next_after("0 9 * * mon-fri", "2025-04-26 10:00"),
            "2025-04-28 09:00"
        );
        assert_eq!(
            next_after("0 0 1 jan *", "2025-04-22 09:00"),
            "2026-01-01 00:00"
        );
    }

    #[test]
    fn expand_basics() {
        assert_eq!(expand("*/15", 0, 59, &[]), Some(vec![0, 15, 30, 45]));
        assert_eq!(expand("1-5", 0, 6, &[]), Some(vec![1, 2, 3, 4, 5]));
        assert_eq!(expand("7", 0, 6, &[]), Some(vec![0])); // Sunday alias
        assert_eq!(expand("5-7", 0, 6, &[]), Some(vec![5, 6]));
        assert_eq!(expand("10-20/5", 0, 59, &[]), Some(vec![10, 15, 20]));
        assert_eq!(expand("99", 0, 59, &[]), None);
        assert_eq!(expand("*/0", 0, 59, &[]), None);
        assert_eq!(expand("", 0, 59, &[]), None);
    }

    #[test]
    fn expand_names() {
        assert_eq!(
            expand("MON-FRI", 0, 6, &WD_ABBREV),
            Some(vec![1, 2, 3, 4, 5])
        );
        assert_eq!(expand("sun,sat", 0, 6, &WD_ABBREV), Some(vec![0, 6]));
        assert_eq!(
            expand("jan,jun,dec", 1, 12, &MONTH_ABBREV),
            Some(vec![1, 6, 12])
        );
        assert_eq!(expand("Feb-Apr", 1, 12, &MONTH_ABBREV), Some(vec![2, 3, 4]));
        assert_eq!(expand("mon", 0, 59, &[]), None); // names not valid for numeric fields
    }

    #[test]
    fn parses_names_in_crontab_lines() {
        let lines = parse_crontab("0 9 * * MON-FRI /bin/true\n");
        assert!(matches!(&lines[0], CrontabLine::Entry(e) if e.enabled));
    }

    #[test]
    fn parse_serialize_round_trip() {
        let content = "SHELL=/bin/sh\n\n# backup job\n0 2 * * * /usr/local/bin/backup\n# 30 3 * * * /old/job\n";
        assert_eq!(serialize_crontab(&parse_crontab(content)), content);
    }

    #[test]
    fn schedule_validation() {
        assert!(sched("0 9 * * mon-fri").is_valid());
        assert!(!sched("99 * * * *").is_valid());
        assert!(!sched("* * * * 8").is_valid());
        assert!(CronSchedule::Special("@daily".into()).is_valid());
        assert!(CronSchedule::Special("@Reboot".into()).is_valid());
        assert!(!CronSchedule::Special("@fortnightly".into()).is_valid());
    }

    #[test]
    fn describes_dom_and_weekday_with_month_step() {
        assert_eq!(
            describe_standard("0", "13", "1", "*/5", "7", true),
            "At 13:00 on the 1st or on Sunday in every 5th month"
        );
    }

    #[test]
    fn describes_dom_and_weekday_as_either_match() {
        assert_eq!(
            describe_standard("0", "13", "1", "*", "7", true),
            "At 13:00 on the 1st or on Sunday"
        );
        assert_eq!(
            describe_standard("5", "4", "1-7", "*", "0", true),
            "At 04:05 on days 1 to 7 or on Sunday"
        );
    }

    #[test]
    fn describes_dom_lists_and_weekday_names() {
        assert_eq!(
            describe_standard("5", "4", "1,15", "*", "*", true),
            "Monthly on the 1st and the 15th at 04:05"
        );
        assert_eq!(
            describe_standard("0", "9", "*", "*", "mon-fri", true),
            "Weekdays at 09:00"
        );
    }
}
