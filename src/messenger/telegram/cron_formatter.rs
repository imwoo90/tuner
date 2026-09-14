//! # Telegram Cron Formatting Helper
//!
//! Provides human-friendly natural language formatting for cron expressions,
//! next execution time calculations, and target chat/topic displays.

use crate::cron::manager::CronJob;
use crate::t;
use super::TopicNameCache;

pub(crate) fn format_target_topic(job: &CronJob, topic_cache: Option<&TopicNameCache>) -> String {
    if job.chat_id == 0 {
        return t!("bot.cron_target_unset");
    }
    if let Some(tid) = job.topic_id {
        if let Some(cache) = topic_cache {
            if let Some(name) = cache.find_by_id(job.chat_id, tid) {
                return format!("<code>#{}</code>", html_escape::encode_safe(&name));
            }
        }
        format!("<code>Topic #{}</code>", tid)
    } else {
        t!("bot.cron_target_main_chat")
    }
}

pub(crate) fn parse_schedule_natural(schedule: &str) -> String {
    let parts: Vec<&str> = schedule.split_whitespace().collect();
    if parts.len() == 5 {
        let (min, hour, day, month, dow) = (parts[0], parts[1], parts[2], parts[3], parts[4]);
        if day == "*" && month == "*" {
            if dow == "*" {
                if let (Ok(m), Ok(h)) = (min.parse::<u32>(), hour.parse::<u32>()) {
                    let time = format!("{:02}:{:02}", h, m);
                    return t!("bot.cron_schedule_daily", time = time);
                } else if let Some(stripped) = min.strip_prefix("*/") {
                    if hour == "*" {
                        return t!("bot.cron_schedule_every_min", n = stripped);
                    }
                } else if min == "0" {
                    if let Some(stripped) = hour.strip_prefix("*/") {
                        return t!("bot.cron_schedule_every_hours", n = stripped);
                    } else if hour == "*" {
                        return t!("bot.cron_schedule_hourly");
                    }
                }
            } else if dow == "1-5" || dow.eq_ignore_ascii_case("mon-fri") {
                if let (Ok(m), Ok(h)) = (min.parse::<u32>(), hour.parse::<u32>()) {
                    let time = format!("{:02}:{:02}", h, m);
                    return t!("bot.cron_schedule_weekdays", time = time);
                }
            } else if dow == "0,6" || dow == "6,0" || dow.eq_ignore_ascii_case("sat,sun") {
                if let (Ok(m), Ok(h)) = (min.parse::<u32>(), hour.parse::<u32>()) {
                    let time = format!("{:02}:{:02}", h, m);
                    return t!("bot.cron_schedule_weekends", time = time);
                }
            }
        }
    }
    format!("<code>{}</code>", html_escape::encode_safe(schedule))
}

pub(crate) fn format_schedule_display(schedule: &str, timezone: &str) -> String {
    let natural = parse_schedule_natural(schedule);
    let next_run_str = match crate::cron::scheduler::calculate_job_next_run(schedule, timezone) {
        Ok(next) => {
            let formatted_time = next.format("%m/%d %H:%M").to_string();
            format!(" {}", t!("bot.cron_next_run", time = format!("<code>{}</code>", formatted_time)))
        }
        Err(_) => String::new(),
    };
    format!("{}{}", natural, next_run_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_schedule_natural_common_patterns() {
        assert!(parse_schedule_natural("0 3 * * *").contains("03:00"));
        assert!(parse_schedule_natural("*/15 * * * *").contains("15"));
        assert!(parse_schedule_natural("0 */2 * * *").contains("2"));
        assert_eq!(parse_schedule_natural("custom * *"), "<code>custom * *</code>");
    }

    #[test]
    fn test_format_schedule_display_includes_next_run() {
        let display = format_schedule_display("0 3 * * *", "Asia/Seoul");
        assert!(display.contains("03:00"));
        assert!(display.contains("Next:") || display.contains("다음:"));
    }
}

