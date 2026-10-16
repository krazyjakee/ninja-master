use super::duration;

/// List columns use at most eight cartridge-font cells. Details keep exact values.
pub(super) fn compact_count(value: u64) -> String {
    for (unit, divisor) in [
        ("Q", 1_000_000_000_000_000),
        ("T", 1_000_000_000_000),
        ("B", 1_000_000_000),
        ("M", 1_000_000),
    ] {
        if value >= divisor {
            return format!(
                "{}.{}{unit}",
                value / divisor,
                (value % divisor) / (divisor / 10)
            );
        }
    }
    value.to_string()
}

pub(super) fn compact_duration(ticks: u64) -> String {
    let hours = ticks / 60 / 3600;
    if hours < 100 {
        return duration(ticks);
    }
    let days = hours / 24;
    if days < 1000 {
        format!("{days}D {:02}H", hours % 24)
    } else {
        format!("{}D", compact_count(days))
    }
}

pub(super) fn wrap(text: &str, width: usize) -> Vec<String> {
    let text: String = text
        .chars()
        .map(|c| {
            if c.is_ascii() {
                c.to_ascii_uppercase()
            } else {
                '?'
            }
        })
        .collect();
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if word.len() > width {
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            for part in word.as_bytes().chunks(width) {
                let part = std::str::from_utf8(part).unwrap().to_owned();
                if part.len() == width {
                    lines.push(part);
                } else {
                    line = part;
                }
            }
        } else {
            if !line.is_empty() && line.len() + 1 + word.len() > width {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

pub(super) fn status_copy(status: &str) -> &str {
    if status.starts_with("PROFILE SAVE ERROR") {
        "YOUR PROFILE COULD NOT BE SAVED. PROGRESS IS KEPT FOR THIS SESSION. CHECK THAT THE FOLDER IS WRITABLE."
    } else if status.starts_with("NEWER PROFILE") {
        "THIS PROFILE NEEDS A NEWER GAME VERSION. THE ORIGINAL FILE IS PRESERVED. NEW PROGRESS CANNOT BE SAVED."
    } else if status.starts_with("PROFILE DAMAGED") {
        "YOUR PROFILE COULD NOT BE READ. THE ORIGINAL FILE IS PRESERVED. NEW PROGRESS CANNOT BE SAVED."
    } else if status.starts_with("PROFILE READ ERROR") {
        "YOUR PROFILE COULD NOT BE OPENED. THE ORIGINAL FILE IS PRESERVED. NEW PROGRESS CANNOT BE SAVED."
    } else {
        status
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn list_values_fit_and_details_can_keep_full_precision() {
        for value in [
            0,
            999_999,
            1_000_000,
            999_999_999,
            1_000_000_000_000_000,
            u64::MAX,
        ] {
            assert!(compact_count(value).len() <= 8, "{value}");
            assert!(compact_duration(value).len() <= 8, "{value}");
        }
        assert_eq!(compact_count(1_250_000), "1.2M");
        assert_eq!(compact_duration(100 * 3600 * 60), "4D 04H");
    }
    #[test]
    fn requirements_and_profile_errors_fit_the_detail_area() {
        for item in crate::profile::CATALOG {
            let lines = wrap(item.description, 36);
            assert!(lines.len() <= 3, "{}: {lines:?}", item.name);
            assert_eq!(lines.join(" "), item.description.to_ascii_uppercase());
        }
        for status in [
            "PROFILE SAVE ERROR",
            "PROFILE READ ERROR",
            "PROFILE DAMAGED",
            "NEWER PROFILE",
        ] {
            assert!(wrap(status_copy(status), 36).len() <= 4);
        }
        let lines = wrap(&"X".repeat(100), 36);
        assert!(lines.iter().all(|line| line.len() <= 36));
        assert_eq!(lines.concat(), "X".repeat(100));
    }
}
