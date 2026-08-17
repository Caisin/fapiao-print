use super::normalize::{clean_name, compact, normalize_credit_code};
use crate::{RecognitionPage, RecognitionWord};
use regex::Regex;

#[derive(Debug, Default)]
pub(crate) struct IdentityFields {
    pub invoice_no: String,
    pub invoice_date: String,
    pub invoice_type: String,
    pub buyer_name: String,
    pub buyer_credit_code: String,
    pub seller_name: String,
    pub seller_credit_code: String,
    pub is_ticket: bool,
    pub is_non_tax: bool,
}

pub(crate) fn extract_identity(text: &str, page: &RecognitionPage) -> IdentityFields {
    let compact_text = compact(text);
    let is_non_tax = ["非税收入", "财政票据", "财政电子票据", "票据号码", "交款人"]
        .iter()
        .any(|keyword| compact_text.contains(keyword));
    let is_ticket = !is_non_tax
        && [
            "铁路电子客票",
            "航空运输电子客票",
            "行程单",
            "旅客运输服务",
            "车次",
        ]
        .iter()
        .any(|keyword| compact_text.contains(keyword));

    let mut result = IdentityFields {
        invoice_type: detect_invoice_type(&compact_text, is_ticket, is_non_tax),
        invoice_no: capture(
            &compact_text,
            r"(?:发票号码|票据号码|发票号|票据号)[:：]?([0-9]{8,20})",
        ),
        invoice_date: extract_date(&compact_text),
        buyer_name: capture_name(text, &["购买方名称", "购买方信息名称", "交款人"]),
        seller_name: capture_name(text, &["销售方名称", "销售方信息名称", "收款单位"]),
        buyer_credit_code: capture_code(
            &compact_text,
            &["购买方统一社会信用代码", "购买方纳税人识别号"],
        ),
        seller_credit_code: capture_code(
            &compact_text,
            &["销售方统一社会信用代码", "销售方纳税人识别号"],
        ),
        is_ticket,
        is_non_tax,
    };

    fill_unqualified_names(&mut result, text);
    fill_from_coordinates(&mut result, page);
    fill_credit_codes(&mut result, &compact_text, page);
    fill_names_from_code_context(&mut result, text);
    if result.invoice_no.is_empty() {
        result.invoice_no = find_likely_invoice_number(text, &result);
    }
    result
}

fn detect_invoice_type(text: &str, is_ticket: bool, is_non_tax: bool) -> String {
    if is_non_tax {
        return "nontax".to_string();
    }
    if text.contains("铁路电子客票") {
        return "train".to_string();
    }
    if text.contains("航空运输电子客票") || text.contains("行程单") {
        return "flight".to_string();
    }
    if is_ticket {
        return "ticket".to_string();
    }
    if text.contains("增值税专用发票") || text.contains("专用发票") {
        return "vat-special".to_string();
    }
    if text.contains("普通发票") || text.contains("电子发票") {
        return "vat-general".to_string();
    }
    "unknown".to_string()
}

fn extract_date(text: &str) -> String {
    for pattern in [
        r"开票日期[:：]?([0-9]{4})年([0-9]{1,2})月([0-9]{1,2})日",
        r"开票日期[:：]?([0-9]{4})[-/]([0-9]{1,2})[-/]([0-9]{1,2})",
        r"([0-9]{4})年([0-9]{1,2})月([0-9]{1,2})日",
        r"([0-9]{4})[-/]([0-9]{1,2})[-/]([0-9]{1,2})",
    ] {
        if let Ok(regex) = Regex::new(pattern) {
            if let Some(captures) = regex.captures(text) {
                return format!("{}-{:0>2}-{:0>2}", &captures[1], &captures[2], &captures[3]);
            }
        }
    }
    String::new()
}

fn capture(text: &str, pattern: &str) -> String {
    Regex::new(pattern)
        .ok()
        .and_then(|regex| regex.captures(text))
        .and_then(|captures| captures.get(1).map(|value| value.as_str().to_string()))
        .unwrap_or_default()
}

fn capture_name(text: &str, labels: &[&str]) -> String {
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let compact_line = compact(line);
        for label in labels {
            if let Some(index) = compact_line.find(label) {
                let value = &compact_line[index + label.len()..];
                let value = value.trim_start_matches([':', '：']);
                let cleaned = clean_name(value);
                if !cleaned.is_empty() {
                    return cleaned;
                }
            }
        }
    }
    String::new()
}

fn capture_code(text: &str, labels: &[&str]) -> String {
    let regex = match Regex::new(r"^[^A-Z0-9]{0,8}([0-9][A-Z0-9]{14,19})") {
        Ok(regex) => regex,
        Err(_) => return String::new(),
    };
    for label in labels {
        if let Some(index) = text.find(label) {
            let tail = &text[index + label.len()..];
            if let Some(captures) = regex.captures(tail) {
                return captures[1].to_string();
            }
        }
    }
    String::new()
}

fn fill_unqualified_names(result: &mut IdentityFields, text: &str) {
    let names = text
        .lines()
        .map(compact)
        .filter_map(|line| {
            line.strip_prefix("名称:")
                .map(clean_name)
                .filter(|name| !name.is_empty())
        })
        .collect::<Vec<_>>();
    if clean_name(&result.buyer_name).is_empty() {
        result.buyer_name = names.first().cloned().unwrap_or_default();
    }
    if clean_name(&result.seller_name).is_empty() {
        result.seller_name = names
            .iter()
            .find(|name| **name != result.buyer_name)
            .cloned()
            .unwrap_or_default();
    }
}

fn fill_from_coordinates(result: &mut IdentityFields, page: &RecognitionPage) {
    if page.img_w == 0 || page.img_h == 0 {
        return;
    }
    let words = flatten_words(page);
    for (index, word) in words.iter().enumerate() {
        let label = compact(&word.text);
        if result.invoice_no.is_empty()
            && (label.contains("发票号码") || label.contains("票据号码"))
        {
            result.invoice_no = nearest_value(&words, index, |value| {
                value.len() >= 8 && value.len() <= 20 && value.chars().all(|ch| ch.is_ascii_digit())
            });
        }
        if (label.contains("名称") || label.contains("交款人") || label.contains("收款单位"))
            && (result.buyer_name.is_empty() || result.seller_name.is_empty())
        {
            let value = nearest_value(&words, index, |candidate| !clean_name(candidate).is_empty());
            let value = clean_name(&value);
            if value.is_empty() {
                continue;
            }
            let is_buyer = label.contains("购买")
                || label.contains("交款")
                || word.x < page.img_w as f64 * 0.5;
            if is_buyer && result.buyer_name.is_empty() {
                result.buyer_name = value;
            } else if !is_buyer && result.seller_name.is_empty() {
                result.seller_name = value;
            }
        }
    }
}

fn fill_credit_codes(result: &mut IdentityFields, text: &str, page: &RecognitionPage) {
    let regex = match Regex::new(r"(?:^|[^A-Z0-9])([0-9][A-Z0-9]{14,19})(?:$|[^A-Z0-9])") {
        Ok(regex) => regex,
        Err(_) => return,
    };
    let mut candidates = regex
        .captures_iter(text)
        .filter_map(|captures| {
            captures
                .get(1)
                .map(|value| normalize_credit_code(value.as_str()))
        })
        .filter(|code| {
            code != &result.invoice_no
                && (!code.chars().all(|ch| ch.is_ascii_digit()) || code.len() == 18)
        })
        .collect::<Vec<_>>();
    for line in &page.lines {
        let joined = line
            .words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<String>();
        for captures in regex.captures_iter(&normalize_credit_code(&joined)) {
            if let Some(value) = captures.get(1) {
                candidates.push(value.as_str().to_string());
            }
        }
    }
    candidates.dedup();
    if result.buyer_credit_code.is_empty() {
        result.buyer_credit_code = candidates.first().cloned().unwrap_or_default();
    }
    if result.seller_credit_code.is_empty() {
        result.seller_credit_code = candidates
            .iter()
            .find(|candidate| **candidate != result.buyer_credit_code)
            .cloned()
            .unwrap_or_default();
    }
}

fn fill_names_from_code_context(result: &mut IdentityFields, text: &str) {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();

    if clean_name(&result.buyer_name).is_empty() {
        result.buyer_name = name_before_code(&lines, &result.buyer_credit_code);
    }
    if clean_name(&result.seller_name).is_empty() {
        result.seller_name = name_before_code(&lines, &result.seller_credit_code);
    }

    if clean_name(&result.buyer_name).is_empty() || clean_name(&result.seller_name).is_empty() {
        let company_names = lines
            .iter()
            .filter_map(|line| {
                let name = clean_name(line);
                (!name.is_empty() && looks_like_company_name(&name)).then_some(name)
            })
            .collect::<Vec<_>>();
        if clean_name(&result.buyer_name).is_empty() {
            result.buyer_name = company_names.first().cloned().unwrap_or_default();
        }
        if clean_name(&result.seller_name).is_empty() {
            result.seller_name = company_names
                .iter()
                .find(|name| **name != result.buyer_name)
                .cloned()
                .unwrap_or_default();
        }
    }
}

fn name_before_code(lines: &[&str], code: &str) -> String {
    if code.is_empty() {
        return String::new();
    }
    let Some(code_index) = lines
        .iter()
        .position(|line| normalize_credit_code(line).contains(code))
    else {
        return String::new();
    };
    lines[..code_index]
        .iter()
        .rev()
        .take(6)
        .map(|line| clean_name(line))
        .find(|name| !name.is_empty() && looks_like_company_name(name))
        .unwrap_or_default()
}

fn looks_like_company_name(value: &str) -> bool {
    [
        "公司",
        "企业",
        "中心",
        "商行",
        "商店",
        "酒店",
        "饭店",
        "餐厅",
        "经营部",
        "合作社",
        "事务所",
    ]
    .iter()
    .any(|suffix| value.contains(suffix))
}

fn find_likely_invoice_number(text: &str, result: &IdentityFields) -> String {
    if let Some(number) = text
        .lines()
        .map(compact)
        .filter(|value| {
            (8..=20).contains(&value.len())
                && value.chars().all(|ch| ch.is_ascii_digit())
                && value != &result.buyer_credit_code
                && value != &result.seller_credit_code
        })
        .max_by_key(String::len)
    {
        return number;
    }
    let regex = match Regex::new(r"(?:^|\D)([0-9]{10,20})(?:$|\D)") {
        Ok(regex) => regex,
        Err(_) => return String::new(),
    };
    regex
        .captures_iter(text)
        .filter_map(|captures| captures.get(1).map(|value| value.as_str()))
        .filter(|value| *value != result.buyer_credit_code && *value != result.seller_credit_code)
        .max_by_key(|value| value.len())
        .unwrap_or_default()
        .to_string()
}

fn flatten_words(page: &RecognitionPage) -> Vec<&RecognitionWord> {
    page.lines
        .iter()
        .flat_map(|line| line.words.iter())
        .collect()
}

fn nearest_value<F>(words: &[&RecognitionWord], label_index: usize, predicate: F) -> String
where
    F: Fn(&str) -> bool,
{
    let label = words[label_index];
    words
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != label_index)
        .filter(|(_, word)| {
            let same_row = (word.y - label.y).abs() <= label.h.max(word.h) * 2.5;
            let nearby_below = word.y >= label.y && word.y - label.y <= label.h.max(1.0) * 4.0;
            (same_row && word.x >= label.x) || nearby_below
        })
        .filter_map(|(_, word)| {
            let value = compact(&word.text);
            predicate(&value).then_some((distance(label, word), value))
        })
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .map(|(_, value)| value)
        .unwrap_or_default()
}

fn distance(left: &RecognitionWord, right: &RecognitionWord) -> f64 {
    let dx = (left.x + left.w * 0.5) - (right.x + right.w * 0.5);
    let dy = (left.y + left.h * 0.5) - (right.y + right.h * 0.5);
    dx * dx + dy * dy
}
