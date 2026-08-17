use crate::{InvoiceLineItem, RecognitionPage};

struct ParsedItemDetails {
    project_name: String,
    specification: String,
    unit: String,
    quantity: Option<f64>,
    unit_price: Option<f64>,
}

pub(crate) fn extract_line_items(
    page: &RecognitionPage,
    text: &str,
    amount_no_tax: f64,
    tax_amount: f64,
    tax_rate: &str,
) -> Vec<InvoiceLineItem> {
    let mut items = Vec::new();
    let mut accepts_continuation = false;
    let mut pending_product = None::<Vec<String>>;
    for line in &page.lines {
        let values = line
            .words
            .iter()
            .map(|word| word.text.trim())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        let Some(rate_index) = values.iter().position(|value| is_tax_rate(value)) else {
            if values.first().is_some_and(|value| value.starts_with('*')) {
                pending_product = Some(values.iter().map(|value| (*value).to_string()).collect());
                accepts_continuation = false;
                continue;
            }
            accepts_continuation =
                accepts_continuation && append_name_continuation(&mut items, &values);
            continue;
        };
        if rate_index == 0 || rate_index + 1 >= values.len() {
            accepts_continuation = false;
            continue;
        }
        let Some(amount) = parse_number(values[rate_index - 1]) else {
            accepts_continuation = false;
            continue;
        };
        let Some(tax_amount) = parse_number(values[rate_index + 1]) else {
            accepts_continuation = false;
            continue;
        };
        let prefix = &values[..rate_index - 1];
        let details = if prefix.first().is_some_and(|value| value.starts_with('*')) {
            pending_product = None;
            parse_inline_product(prefix)
        } else if let Some(pending) = pending_product.take() {
            parse_pending_product(&pending, prefix)
        } else {
            None
        };
        let Some(details) = details else {
            accepts_continuation = false;
            continue;
        };
        items.push(InvoiceLineItem {
            project_name: details.project_name,
            specification: details.specification,
            unit: details.unit,
            quantity: details.quantity,
            unit_price: details.unit_price,
            amount,
            tax_rate: values[rate_index].to_string(),
            tax_amount,
            amount_tax: round_money(amount + tax_amount),
            is_discount: amount < 0.0 || tax_amount < 0.0,
        });
        accepts_continuation = true;
    }
    fill_discount_names(&mut items);
    if items.len() == 1
        && amount_no_tax > 0.0
        && tax_amount >= 0.0
        && ((items[0].amount - amount_no_tax).abs() > 0.02
            || (items[0].tax_amount - tax_amount).abs() > 0.02)
    {
        items.clear();
    }
    if items.is_empty() {
        return extract_single_packed_item(text, amount_no_tax, tax_amount, tax_rate);
    }
    items
}

fn parse_inline_product(prefix: &[&str]) -> Option<ParsedItemDetails> {
    if prefix.is_empty() || !prefix[0].starts_with('*') {
        return None;
    }
    Some(if prefix.len() >= 5 {
        let split = prefix.len() - 4;
        ParsedItemDetails {
            project_name: prefix[..split].join(""),
            specification: prefix[split].to_string(),
            unit: prefix[split + 1].to_string(),
            quantity: parse_number(prefix[split + 2]),
            unit_price: parse_number(prefix[split + 3]),
        }
    } else {
        ParsedItemDetails {
            project_name: prefix.join(""),
            specification: String::new(),
            unit: String::new(),
            quantity: None,
            unit_price: None,
        }
    })
}

fn parse_pending_product(pending: &[String], numeric_prefix: &[&str]) -> Option<ParsedItemDetails> {
    if pending.is_empty() || !pending[0].starts_with('*') {
        return None;
    }
    let has_unit = pending
        .last()
        .is_some_and(|value| pending.len() > 1 && looks_like_unit(value));
    let project_end = pending.len() - usize::from(has_unit);
    let project_name = pending[..project_end].join("");
    let unit = has_unit
        .then(|| pending.last().cloned())
        .flatten()
        .unwrap_or_default();
    Some(ParsedItemDetails {
        project_name,
        specification: String::new(),
        unit,
        quantity: numeric_prefix.first().and_then(|value| parse_number(value)),
        unit_price: numeric_prefix.get(1).and_then(|value| parse_number(value)),
    })
}

fn looks_like_unit(value: &str) -> bool {
    let count = value.chars().count();
    (1..=4).contains(&count)
        && value
            .chars()
            .all(|character| !character.is_ascii_digit() && !".*%¥￥".contains(character))
}

fn extract_single_packed_item(
    text: &str,
    amount_no_tax: f64,
    tax_amount: f64,
    tax_rate: &str,
) -> Vec<InvoiceLineItem> {
    if amount_no_tax <= 0.0 || tax_rate.contains(',') {
        return Vec::new();
    }
    let mut projects = text
        .split(['\t', '\n', '\r'])
        .map(str::trim)
        .filter(|value| {
            value.starts_with('*') && value[1..].contains('*') && !value.contains("项目名称")
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    if projects.is_empty() {
        projects.extend(text.lines().filter_map(extract_compact_project));
    }
    projects.dedup();
    if projects.len() != 1 {
        return Vec::new();
    }
    vec![InvoiceLineItem {
        project_name: projects[0].clone(),
        amount: amount_no_tax,
        tax_rate: tax_rate.to_string(),
        tax_amount,
        amount_tax: round_money(amount_no_tax + tax_amount),
        ..Default::default()
    }]
}

fn extract_compact_project(line: &str) -> Option<String> {
    let compact = line
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let start = compact.find('*')?;
    let second = compact[start + 1..].find('*')? + start + 1;
    let tail = &compact[second + 1..];
    let end = tail
        .find('*')
        .map(|index| second + 1 + index)
        .or_else(|| {
            tail.find(|character: char| character.is_ascii_digit() || character == '-')
                .map(|index| second + 1 + index)
        })
        .unwrap_or(compact.len());
    let mut project = compact[start..end].to_string();
    for suffix in ["无次", "无", "次"] {
        if project.ends_with(suffix) {
            project.truncate(project.len() - suffix.len());
            break;
        }
    }
    (project.len() > 2).then_some(project)
}

fn fill_discount_names(items: &mut [InvoiceLineItem]) {
    for index in 1..items.len() {
        if !items[index].is_discount {
            continue;
        }
        let previous = &items[index - 1];
        if !previous.is_discount
            && previous
                .project_name
                .starts_with(&items[index].project_name)
        {
            items[index].project_name = previous.project_name.clone();
        }
    }
}

fn append_name_continuation(items: &mut [InvoiceLineItem], values: &[&str]) -> bool {
    if values.len() != 1 || values[0].starts_with('*') || parse_number(values[0]).is_some() {
        return false;
    }
    if let Some(item) = items.last_mut() {
        item.project_name.push_str(values[0]);
        return true;
    }
    false
}

fn is_tax_rate(value: &str) -> bool {
    let value = value.trim();
    (value.ends_with('%')
        && value
            .trim_end_matches('%')
            .parse::<f64>()
            .is_ok_and(|rate| (0.0..=100.0).contains(&rate)))
        || matches!(value, "免税" | "不征税" | "零税率")
}

fn parse_number(value: &str) -> Option<f64> {
    value
        .trim()
        .trim_start_matches(['¥', '￥'])
        .replace(',', "")
        .parse::<f64>()
        .ok()
}

fn round_money(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RecognitionLine, RecognitionWord};

    #[test]
    fn extracts_regular_discount_and_continued_product_rows() {
        let page = RecognitionPage {
            lines: vec![
                line(&[
                    "*其他食品*青豌豆小辣丁",
                    "11024750",
                    "袋",
                    "2",
                    "0.885",
                    "1.77",
                    "13%",
                    "0.23",
                ]),
                line(&["20g"]),
                line(&["*其他食品*青豌豆小辣丁", "-0.02", "13%", "0.00"]),
                line(&["20g"]),
            ],
            ..Default::default()
        };

        let items = extract_line_items(&page, &page.text, 0.0, 0.0, "");

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].project_name, "*其他食品*青豌豆小辣丁20g");
        assert_eq!(items[0].specification, "11024750");
        assert_eq!(items[0].quantity, Some(2.0));
        assert_eq!(items[0].unit_price, Some(0.885));
        assert_eq!(items[0].amount_tax, 2.0);
        assert!(!items[0].is_discount);
        assert_eq!(items[1].project_name, "*其他食品*青豌豆小辣丁20g");
        assert_eq!(items[1].amount, -0.02);
        assert!(items[1].is_discount);
    }

    #[test]
    fn joins_product_header_with_following_numeric_row() {
        let page = RecognitionPage {
            lines: vec![
                line(&["*玩具*玩具乐器", "套"]),
                line(&["1", "353.1", "353.10", "13%", "45.90"]),
                line(&["*玩具*玩具乐器", "-137.61", "13%", "-17.89"]),
            ],
            ..Default::default()
        };

        let items = extract_line_items(&page, &page.text, 215.49, 28.01, "13%");

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].project_name, "*玩具*玩具乐器");
        assert_eq!(items[0].unit, "套");
        assert_eq!(items[0].quantity, Some(1.0));
        assert_eq!(items[0].unit_price, Some(353.1));
        assert_eq!(items[0].amount, 353.1);
        assert_eq!(items[0].tax_amount, 45.9);
        assert_eq!(items[1].amount, -137.61);
        assert_eq!(items[1].tax_amount, -17.89);
    }

    fn line(values: &[&str]) -> RecognitionLine {
        RecognitionLine {
            words: values
                .iter()
                .enumerate()
                .map(|(index, value)| RecognitionWord {
                    text: (*value).to_string(),
                    x: index as f64 * 100.0,
                    ..Default::default()
                })
                .collect(),
            confidence: 1.0,
        }
    }
}
