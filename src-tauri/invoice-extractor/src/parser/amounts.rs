use super::normalize::compact;
use super::{round_money, valid_tax_rate};
use regex::Regex;

#[derive(Debug, Default)]
pub(crate) struct Amounts {
    pub amount_tax: f64,
    pub amount_no_tax: f64,
    pub tax_amount: f64,
    pub tax_rate: String,
    pub amount_uppercase: String,
}

pub(crate) fn extract_amounts(text: &str, is_ticket: bool, is_non_tax: bool) -> Amounts {
    let compact_text = compact(text);
    let all = collect_amounts(&compact_text);
    let mut result = Amounts {
        amount_tax: first_amount_after(
            &compact_text,
            &[
                "价税合计(小写)",
                "价税合计（小写）",
                "金额合计(小写)",
                "金额合计（小写）",
                "价税合计",
                "金额合计",
                "票价",
                "实付",
                "应付",
            ],
        ),
        ..Default::default()
    };
    result.amount_uppercase = extract_chinese_total_text(&compact_text);
    if result.amount_tax <= 0.0 {
        result.amount_tax = parse_chinese_number(&result.amount_uppercase);
    }
    if result.amount_tax <= 0.0 {
        result.amount_tax = all.iter().copied().fold(0.0, f64::max);
    }

    if is_non_tax {
        result.amount_no_tax = result.amount_tax;
        result.tax_rate = extract_tax_rate(text, &result, is_ticket, is_non_tax);
        return result;
    }

    result.tax_amount = first_amount_after(&compact_text, &["税额合计", "税额"]);
    result.amount_no_tax = first_amount_after(&compact_text, &["不含税金额", "金额合计", "合计"]);

    if result.amount_no_tax >= result.amount_tax {
        result.amount_no_tax = 0.0;
    }
    if result.tax_amount >= result.amount_tax {
        result.tax_amount = 0.0;
    }
    if !is_ticket {
        fill_by_math(&mut result, &all);
    }
    if result.amount_tax > 0.0 && result.amount_no_tax <= 0.0 && result.tax_amount > 0.0 {
        let candidate = round_money(result.amount_tax - result.tax_amount);
        if valid_tax_rate(candidate, result.tax_amount) {
            result.amount_no_tax = candidate;
        }
    }
    if result.amount_tax > 0.0 && result.tax_amount <= 0.0 && result.amount_no_tax > 0.0 {
        let candidate = round_money(result.amount_tax - result.amount_no_tax);
        if valid_tax_rate(result.amount_no_tax, candidate) {
            result.tax_amount = candidate;
        }
    }
    if result.amount_tax > 0.0 && result.amount_no_tax <= 0.0 && result.tax_amount <= 0.0 {
        result.amount_no_tax = result.amount_tax;
    }
    result.tax_rate = extract_tax_rate(text, &result, is_ticket, is_non_tax);
    result
}

fn extract_tax_rate(text: &str, amounts: &Amounts, is_ticket: bool, is_non_tax: bool) -> String {
    let mut candidates = Vec::<(usize, String)>::new();
    if let Ok(regex) = Regex::new(r"([0-9]{1,3}(?:\.[0-9]{1,4})?)%") {
        for captures in regex.captures_iter(text) {
            if let (Some(full), Some(value)) = (captures.get(0), captures.get(1)) {
                if let Ok(percent) = value.as_str().parse::<f64>() {
                    if percent <= 100.0 {
                        candidates.push((full.start(), format_percent(percent)));
                    }
                }
            }
        }
    }
    for label in ["不征税", "免税", "零税率"] {
        for (position, _) in text.match_indices(label) {
            candidates.push((position, label.to_string()));
        }
    }
    candidates.sort_by_key(|(position, _)| *position);
    let mut rates = Vec::new();
    for (_, rate) in candidates {
        push_unique(&mut rates, rate);
    }
    if rates.is_empty()
        && !is_ticket
        && !is_non_tax
        && amounts.amount_no_tax > 0.0
        && amounts.tax_amount > 0.0
    {
        let percent = amounts.tax_amount / amounts.amount_no_tax * 100.0;
        if percent <= 100.0 {
            push_unique(&mut rates, format_derived_percent(percent));
        }
    }
    rates.join(",")
}

pub(crate) fn normalize_tax_rate(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    if ["不征税", "免税", "零税率"]
        .iter()
        .any(|label| value.contains(label))
    {
        return ["不征税", "免税", "零税率"]
            .iter()
            .find(|label| value.contains(**label))
            .unwrap()
            .to_string();
    }
    let numeric = value.trim_end_matches('%').replace(',', "").parse::<f64>();
    let Ok(mut percent) = numeric else {
        return String::new();
    };
    if !value.ends_with('%') && percent > 0.0 && percent < 1.0 {
        percent *= 100.0;
    }
    if !(0.0..=100.0).contains(&percent) {
        return String::new();
    }
    format_percent(percent)
}

fn format_percent(percent: f64) -> String {
    let rounded = (percent * 10_000.0).round() / 10_000.0;
    if (rounded - rounded.round()).abs() < 0.000_001 {
        format!("{:.0}%", rounded)
    } else {
        let mut value = format!("{rounded:.4}");
        while value.ends_with('0') {
            value.pop();
        }
        format!("{value}%")
    }
}

fn format_derived_percent(percent: f64) -> String {
    let standard_rates = [0.0, 1.0, 3.0, 5.0, 6.0, 9.0, 13.0];
    let closest = standard_rates
        .iter()
        .min_by(|left, right| (*left - percent).abs().total_cmp(&(*right - percent).abs()))
        .copied()
        .unwrap_or(percent);
    if (closest - percent).abs() <= 0.5 {
        format_percent(closest)
    } else {
        format_percent(percent)
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !value.is_empty() && !values.contains(&value) {
        values.push(value);
    }
}

fn first_amount_after(text: &str, labels: &[&str]) -> f64 {
    for label in labels {
        if let Some(index) = text.find(label) {
            let tail = &text[index + label.len()..];
            if let Some(value) =
                collect_amounts(&tail.chars().take(100).collect::<String>()).first()
            {
                return *value;
            }
        }
    }
    0.0
}

fn collect_amounts(text: &str) -> Vec<f64> {
    let regex = match Regex::new(r"(?:¥|￥|RMB)?([0-9]{1,12}(?:,[0-9]{3})*\.[0-9]{2})") {
        Ok(regex) => regex,
        Err(_) => return Vec::new(),
    };
    let mut values = regex
        .captures_iter(text)
        .filter_map(|captures| captures.get(1))
        .filter_map(|value| value.as_str().replace(',', "").parse::<f64>().ok())
        .filter(|value| *value > 0.0 && *value < 1_000_000_000.0)
        .map(round_money)
        .collect::<Vec<_>>();
    values.dedup_by(|left, right| (*left - *right).abs() < 0.001);
    values
}

fn fill_by_math(result: &mut Amounts, values: &[f64]) {
    if result.amount_tax <= 0.0 {
        return;
    }
    let mut best = None::<(f64, f64)>;
    for (left_index, left) in values.iter().enumerate() {
        for right in values.iter().skip(left_index + 1) {
            let larger = left.max(*right);
            let smaller = left.min(*right);
            if (round_money(larger + smaller) - result.amount_tax).abs() <= 0.02
                && larger < result.amount_tax
                && valid_tax_rate(larger, smaller)
                && best.map_or(true, |current| larger > current.0)
            {
                best = Some((larger, smaller));
            }
        }
    }
    if let Some((no_tax, tax)) = best {
        result.amount_no_tax = no_tax;
        result.tax_amount = tax;
    }
}

fn extract_chinese_total_text(text: &str) -> String {
    let regex = match Regex::new(
        r"(?:价税合计|金额合计)?.{0,10}(?:大写)?[):：）]?([零壹贰叁肆伍陆柒捌玖拾佰仟万亿萬億圆元角分整正一二三四五六七八九十]{3,})",
    ) {
        Ok(regex) => regex,
        Err(_) => return String::new(),
    };
    regex
        .captures(text)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().to_string())
        .unwrap_or_default()
}

fn parse_chinese_number(input: &str) -> f64 {
    let normalized = input
        .replace('萬', "万")
        .replace('億', "亿")
        .replace('圆', "元");
    let integer = normalized.split('元').next().unwrap_or("");
    let mut total = 0_u64;
    let mut section = 0_u64;
    let mut digit = 0_u64;
    for ch in integer.chars() {
        if let Some(value) = chinese_digit(ch) {
            digit = value;
        } else if let Some(unit) = small_unit(ch) {
            section += if digit == 0 { unit } else { digit * unit };
            digit = 0;
        } else if ch == '万' {
            total += (section + digit) * 10_000;
            section = 0;
            digit = 0;
        } else if ch == '亿' {
            total = (total + section + digit) * 100_000_000;
            section = 0;
            digit = 0;
        }
    }
    let mut value = (total + section + digit) as f64;
    if let Some(yuan_index) = normalized.find('元') {
        let fraction = &normalized[yuan_index + '元'.len_utf8()..];
        let chars = fraction.chars().collect::<Vec<_>>();
        for (index, ch) in chars.iter().enumerate() {
            if let Some(number) = chinese_digit(*ch) {
                if chars.get(index + 1) == Some(&'角') {
                    value += number as f64 / 10.0;
                }
                if chars.get(index + 1) == Some(&'分') {
                    value += number as f64 / 100.0;
                }
            }
        }
    }
    round_money(value)
}

fn chinese_digit(ch: char) -> Option<u64> {
    match ch {
        '零' => Some(0),
        '壹' | '一' => Some(1),
        '贰' | '二' => Some(2),
        '叁' | '三' => Some(3),
        '肆' | '四' => Some(4),
        '伍' | '五' => Some(5),
        '陆' | '六' => Some(6),
        '柒' | '七' => Some(7),
        '捌' | '八' => Some(8),
        '玖' | '九' => Some(9),
        _ => None,
    }
}

fn small_unit(ch: char) -> Option<u64> {
    match ch {
        '拾' | '十' => Some(10),
        '佰' | '百' => Some(100),
        '仟' | '千' => Some(1_000),
        _ => None,
    }
}
