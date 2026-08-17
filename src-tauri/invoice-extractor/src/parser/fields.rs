use super::normalize::{clean_name, compact, normalize_credit_code};
use crate::{RecognitionLine, RecognitionPage, RecognitionWord};
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
    pub invoice_clerk: String,
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
        invoice_clerk: extract_invoice_clerk(text),
        is_ticket,
        is_non_tax,
    };

    fill_names_from_coordinates(&mut result, page);
    fill_unqualified_names(&mut result, text);
    fill_from_coordinates(&mut result, page);
    fill_credit_codes(&mut result, &compact_text, page);
    fill_names_from_code_context(&mut result, text);
    if result.invoice_no.is_empty() {
        result.invoice_no = find_likely_invoice_number(text, &result);
    }
    repair_tab_packed_fields(&mut result, text);
    result
}

fn fill_names_from_coordinates(result: &mut IdentityFields, page: &RecognitionPage) {
    if page.img_w == 0 {
        return;
    }
    let middle = page.img_w as f64 * 0.5;
    for line in &page.lines {
        let (left, right) = column_texts(line, middle);
        if !is_valid_company_field(&result.buyer_name) {
            if let Some(name) = positioned_company_name(&left) {
                result.buyer_name = name;
            }
        }
        if !is_valid_company_field(&result.seller_name) {
            if let Some(name) = positioned_company_name(&right) {
                result.seller_name = name;
            }
        }
    }
    for word in page.lines.iter().flat_map(|line| &line.words) {
        let line = compact(&word.text);
        let name = name_value(&line)
            .map(clean_positioned_name)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| clean_positioned_name(&line));
        if name.is_empty() {
            continue;
        }
        if name_value(&line).is_none() && !looks_like_company_name(&name) {
            continue;
        }
        if word.x < middle && !is_valid_company_field(&result.buyer_name) {
            result.buyer_name = name;
        } else if word.x >= middle && !is_valid_company_field(&result.seller_name) {
            result.seller_name = name;
        }
    }
}

fn positioned_company_name(value: &str) -> Option<String> {
    let value = compact(value);
    let name = name_value(&value)
        .map(clean_positioned_name)
        .filter(|name| is_valid_company_field(name))
        .or_else(|| {
            let name = clean_positioned_name(&value);
            is_valid_company_field(&name).then_some(name)
        })?;
    Some(name)
}

fn clean_positioned_name(value: &str) -> String {
    let mut value = value.to_string();
    for marker in ["售名称", "销名称", "销售方", "售方"] {
        if let Some(index) = value.find(marker) {
            value.truncate(index);
        }
    }
    if value.ends_with('售') || value.ends_with('销') {
        value.pop();
    }
    let cleaned = clean_name(&value);
    if let Some(without_role_tail) = cleaned.strip_prefix('息') {
        if is_valid_company_field(without_role_tail) {
            return without_role_tail.to_string();
        }
    }
    cleaned
}

fn column_texts(line: &RecognitionLine, middle: f64) -> (String, String) {
    let mut left = Vec::new();
    let mut right = Vec::new();
    for word in &line.words {
        if word.x < middle {
            left.push(word.text.as_str());
        } else {
            right.push(word.text.as_str());
        }
    }
    (join_column_words(&left), join_column_words(&right))
}

fn join_column_words(words: &[&str]) -> String {
    let mut start = 0;
    let mut end = words.len();
    if words.len() > 1
        && words.first().is_some_and(|word| {
            matches!(
                compact(word).as_str(),
                "购" | "买" | "销" | "售" | "方" | "信" | "息"
            )
        })
    {
        start += 1;
    }
    if end > start
        && words
            .last()
            .is_some_and(|word| matches!(compact(word).as_str(), "销" | "售"))
    {
        end -= 1;
    }
    words[start..end].concat()
}

fn name_value(line: &str) -> Option<&str> {
    if let Some(label_index) = line.find("名称") {
        return Some(line[label_index + "名称".len()..].trim_start_matches([':', '：']));
    }
    if line.starts_with('名') {
        return line
            .char_indices()
            .find(|(_, character)| matches!(character, ':' | '：'))
            .map(|(separator, character)| &line[separator + character.len_utf8()..]);
    }
    None
}

fn detect_invoice_type(text: &str, is_ticket: bool, is_non_tax: bool) -> String {
    if is_non_tax {
        return "nontax".to_string();
    }
    if text.contains("增值税专用发票") || text.contains("专用发票") {
        return "vat-special".to_string();
    }
    if text.contains("普通发票") || text.contains("电子普通发票") {
        return "vat-general".to_string();
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
    if text.contains("电子发票") {
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

fn extract_invoice_clerk(text: &str) -> String {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(compact)
        .collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate() {
        for label in ["开票人", "开票员", "InvoiceClerk", "Drawer", "Issuer"] {
            if line.starts_with(label) {
                let value = line.rsplit(label).next().unwrap_or_default();
                let value = value.trim_start_matches([':', '：']);
                if is_person_name(value) {
                    return value.to_string();
                }
                if value.is_empty() {
                    if let Some(value) = lines.get(index + 1).filter(|value| is_person_name(value))
                    {
                        return value.clone();
                    }
                }
            }
        }
    }

    let Some(total_index) = lines.iter().position(|line| {
        line.chars()
            .filter(|ch| is_financial_character(*ch))
            .count()
            >= 3
            && (line.contains('圆') || line.contains('元'))
    }) else {
        return String::new();
    };
    lines
        .iter()
        .skip(total_index + 1)
        .take(8)
        .find(|line| is_person_name(line))
        .cloned()
        .unwrap_or_default()
}

fn is_person_name(value: &str) -> bool {
    let count = value.chars().count();
    (2..=6).contains(&count)
        && ![
            "金额",
            "税额",
            "名称",
            "单位",
            "数量",
            "单价",
            "合计",
            "备注",
            "日期",
            "信息",
            "开票人",
            "开票员",
            "购买方",
            "销售方",
            "项目名称",
            "规格型号",
        ]
        .contains(&value)
        && value
            .chars()
            .all(|ch| matches!(ch as u32, 0x3400..=0x9fff | 0xf900..=0xfaff) || ch == '·')
}

fn repair_tab_packed_fields(result: &mut IdentityFields, text: &str) {
    if !text.contains('\t') {
        return;
    }

    let tokens = text
        .split(['\t', '\n', '\r'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let mut companies = tokens
        .iter()
        .map(|value| clean_name(&compact(value)))
        .filter(|value| is_valid_company_field(value))
        .collect::<Vec<_>>();
    companies.dedup();

    let mut codes = tokens
        .iter()
        .map(|value| normalize_credit_code(value))
        .filter(|value| is_likely_credit_code(value) && value != &result.invoice_no)
        .collect::<Vec<_>>();
    codes.dedup();

    let compact_text = compact(text);
    let has_ordered_party_labels = compact_text.contains("购买方")
        && compact_text.contains("销售方")
        && companies.len() >= 2
        && codes.len() >= 2
        && companies[0] != companies[1]
        && codes[0] != codes[1];
    if has_ordered_party_labels {
        result.buyer_name = companies[0].clone();
        result.seller_name = companies[1].clone();
        result.buyer_credit_code = codes[0].clone();
        result.seller_credit_code = codes[1].clone();
    }

    if !is_valid_company_field(&result.buyer_name) {
        result.buyer_name = company_for_code(&companies, &codes, &result.buyer_credit_code)
            .or_else(|| company_other_than(&companies, &result.seller_name))
            .unwrap_or_default();
    }
    if !is_valid_company_field(&result.seller_name) {
        result.seller_name = company_for_code(&companies, &codes, &result.seller_credit_code)
            .or_else(|| company_other_than(&companies, &result.buyer_name))
            .unwrap_or_default();
    }

    if result.buyer_credit_code.is_empty() {
        result.buyer_credit_code = code_for_company(
            &companies,
            &codes,
            &result.buyer_name,
            &result.seller_credit_code,
        );
    }
    if result.seller_credit_code.is_empty() {
        result.seller_credit_code = code_for_company(
            &companies,
            &codes,
            &result.seller_name,
            &result.buyer_credit_code,
        );
    }

    if result.invoice_clerk.is_empty() {
        result.invoice_clerk = repeated_person_name(&tokens);
    }
}

fn is_valid_company_field(value: &str) -> bool {
    let count = value.chars().count();
    (2..=80).contains(&count)
        && !value.starts_with('*')
        && !value.contains(['\t', '\n', '\r'])
        && !["税务局", "国家税务总局", "税务机关"]
            .iter()
            .any(|label| value.contains(label))
        && looks_like_company_name(value)
        && ![
            "统一社会信用代码",
            "纳税人识别号",
            "项目名称",
            "发票号码",
            "开票日期",
            "税率/征收率",
        ]
        .iter()
        .any(|label| value.contains(label))
}

fn is_likely_credit_code(value: &str) -> bool {
    (15..=20).contains(&value.len())
        && value.starts_with(|character: char| character.is_ascii_digit())
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
        && (value.len() == 18
            || value
                .chars()
                .any(|character| character.is_ascii_alphabetic()))
}

fn company_for_code(companies: &[String], codes: &[String], code: &str) -> Option<String> {
    let index = codes.iter().position(|candidate| candidate == code)?;
    companies.get(index).cloned()
}

fn company_other_than(companies: &[String], other: &str) -> Option<String> {
    companies
        .iter()
        .find(|value| value.as_str() != other)
        .cloned()
}

fn code_for_company(
    companies: &[String],
    codes: &[String],
    company: &str,
    other_code: &str,
) -> String {
    companies
        .iter()
        .position(|candidate| candidate == company)
        .and_then(|index| codes.get(index))
        .filter(|code| code.as_str() != other_code)
        .cloned()
        .or_else(|| {
            codes
                .iter()
                .find(|code| code.as_str() != other_code)
                .cloned()
        })
        .unwrap_or_default()
}

fn repeated_person_name(tokens: &[&str]) -> String {
    let mut candidates = tokens
        .iter()
        .filter(|value| is_person_name(value))
        .copied()
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates
        .windows(2)
        .find(|pair| pair[0] == pair[1])
        .map(|pair| pair[0].to_string())
        .unwrap_or_default()
}

fn is_financial_character(ch: char) -> bool {
    "零壹贰叁肆伍陆柒捌玖拾佰仟万亿萬億圆元角分整正一二三四五六七八九十".contains(ch)
}

fn fill_unqualified_names(result: &mut IdentityFields, text: &str) {
    let names = text
        .lines()
        .flat_map(company_names_after_labels)
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

fn company_names_after_labels(line: &str) -> Vec<String> {
    let line = compact(line);
    let labels = line
        .match_indices("名称")
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let mut names = Vec::new();
    for (position, label_index) in labels.iter().copied().enumerate() {
        let start = label_index + "名称".len();
        let end = labels.get(position + 1).copied().unwrap_or(line.len());
        let value = line[start..end].trim_start_matches([':', '：']);
        let name = clean_positioned_name(value);
        if is_valid_company_field(&name) && !names.contains(&name) {
            names.push(name);
        }
    }
    names
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
    if page.img_w > 0 {
        let middle = page.img_w as f64 * 0.5;
        for line in &page.lines {
            let (left, right) = column_texts(line, middle);
            if result.buyer_credit_code.is_empty() {
                result.buyer_credit_code = credit_code_in_text(&regex, &left, &result.invoice_no);
            }
            if result.seller_credit_code.is_empty() {
                result.seller_credit_code = credit_code_in_text(&regex, &right, &result.invoice_no);
            }
        }
        for word in page.lines.iter().flat_map(|line| &line.words) {
            let normalized = normalize_credit_code(&word.text);
            let Some(code) = regex
                .captures(&normalized)
                .and_then(|captures| captures.get(1))
                .map(|value| value.as_str().to_string())
            else {
                continue;
            };
            if code == result.invoice_no
                || (code.chars().all(|character| character.is_ascii_digit()) && code.len() != 18)
            {
                continue;
            }
            if word.x < middle && result.buyer_credit_code.is_empty() {
                result.buyer_credit_code = code;
            } else if word.x >= middle && result.seller_credit_code.is_empty() {
                result.seller_credit_code = code;
            }
        }
    }
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

fn credit_code_in_text(regex: &Regex, text: &str, invoice_no: &str) -> String {
    let normalized = normalize_credit_code(text);
    regex
        .captures(&normalized)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str())
        .filter(|code| {
            *code != invoice_no
                && (!code.chars().all(|character| character.is_ascii_digit()) || code.len() == 18)
        })
        .unwrap_or_default()
        .to_string()
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
    let contains_entity_marker = [
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
        "个体工商",
    ]
    .iter()
    .any(|marker| value.contains(marker));
    contains_entity_marker || value.ends_with('店') || value.ends_with("餐馆")
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
