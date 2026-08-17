use crate::{InvoiceLineItem, RecognitionPage};

pub(crate) fn extract_line_items(
    page: &RecognitionPage,
    text: &str,
    amount_no_tax: f64,
    tax_amount: f64,
    tax_rate: &str,
) -> Vec<InvoiceLineItem> {
    let mut items = Vec::new();
    let mut accepts_continuation = false;
    for line in &page.lines {
        let values = line
            .words
            .iter()
            .map(|word| word.text.trim())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        let Some(rate_index) = values.iter().position(|value| is_tax_rate(value)) else {
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
        if prefix.is_empty() || !prefix[0].starts_with('*') {
            accepts_continuation = false;
            continue;
        }

        let (project_name, specification, unit, quantity, unit_price) = if prefix.len() >= 5 {
            let split = prefix.len() - 4;
            (
                prefix[..split].join(""),
                prefix[split].to_string(),
                prefix[split + 1].to_string(),
                parse_number(prefix[split + 2]),
                parse_number(prefix[split + 3]),
            )
        } else {
            (prefix.join(""), String::new(), String::new(), None, None)
        };
        items.push(InvoiceLineItem {
            project_name,
            specification,
            unit,
            quantity,
            unit_price,
            amount,
            tax_rate: values[rate_index].to_string(),
            tax_amount,
            amount_tax: round_money(amount + tax_amount),
            is_discount: amount < 0.0 || tax_amount < 0.0,
        });
        accepts_continuation = true;
    }
    fill_discount_names(&mut items);
    if items.is_empty() {
        return extract_single_packed_item(text, amount_no_tax, tax_amount, tax_rate);
    }
    items
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
        .collect::<Vec<_>>();
    projects.dedup();
    if projects.len() != 1 {
        return Vec::new();
    }
    vec![InvoiceLineItem {
        project_name: projects[0].to_string(),
        amount: amount_no_tax,
        tax_rate: tax_rate.to_string(),
        tax_amount,
        amount_tax: round_money(amount_no_tax + tax_amount),
        ..Default::default()
    }]
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
