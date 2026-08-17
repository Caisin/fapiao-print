use super::ParsedFormat;
use crate::parser::parse_recognition_page;
use crate::RecognitionPage;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::path::Path;

pub(crate) fn read_xml(path: &Path, include_raw_text: bool) -> Result<ParsedFormat, String> {
    let content =
        std::fs::read_to_string(path).map_err(|error| format!("读取 XML 文件失败: {error}"))?;
    parse_xml_content(&content, include_raw_text)
}

pub(crate) fn parse_xml_content(
    content: &str,
    include_raw_text: bool,
) -> Result<ParsedFormat, String> {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);
    let mut path = Vec::<String>::new();
    let mut text_parts = Vec::<String>::new();
    let mut info = crate::InvoiceInfo::default();
    let mut invoice_type = String::new();
    let mut vat_type = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                path.push(local_name(event.name().as_ref()));
            }
            Ok(Event::End(_)) => {
                path.pop();
            }
            Ok(Event::Text(event)) => {
                let value = event
                    .unescape()
                    .map_err(|error| format!("XML 文本解码失败: {error}"))?
                    .trim()
                    .to_string();
                if value.is_empty() {
                    continue;
                }
                text_parts.push(value.clone());
                let tag = path.last().map(String::as_str).unwrap_or("");
                let parent = path.iter().rev().nth(1).map(String::as_str).unwrap_or("");
                match tag {
                    "InvoiceNumber" | "InvoiceNo" => info.invoice_no = value,
                    "IssueTime" | "IssueDate" | "InvoiceDate" => {
                        info.invoice_date = value.split('T').next().unwrap_or(&value).to_string();
                    }
                    "SellerName" => info.seller_name = value,
                    "SellerIdNum" | "SellerTaxID" => info.seller_credit_code = value,
                    "BuyerName" => info.buyer_name = value,
                    "BuyerIdNum" | "BuyerTaxID" => info.buyer_credit_code = value,
                    "TotalAmWithoutTax" | "TaxExclusiveTotalAmount" => {
                        info.amount_no_tax = parse_number(&value);
                    }
                    "TotalTaxAm" | "TaxTotalAmount" | "TaxAmount" => {
                        info.tax_amount = parse_number(&value);
                    }
                    "TaxRate" | "TaxRateValue" | "TaxRatePercent" => {
                        let rate = crate::parser::normalize_tax_rate(&value);
                        if info.tax_rate.is_empty() {
                            info.tax_rate = rate;
                        }
                    }
                    "TotalTax-includedAmount"
                    | "TotalTaxIncludedAmount"
                    | "TaxInclusiveTotalAmount"
                    | "Amount" => {
                        info.amount_tax = parse_number(&value);
                    }
                    "LabelName" if parent == "EInvoiceType" => invoice_type = value,
                    "LabelName" if parent == "GeneralOrSpecialVAT" => vat_type = value,
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(format!("XML 解析失败: {error}")),
            _ => {}
        }
    }

    let full_text = text_parts.join("\n");
    let parsed = parse_recognition_page(
        &RecognitionPage::from_text(full_text.clone()),
        0,
        "xml",
        include_raw_text,
    );
    info.merge_missing(parsed);
    if info.invoice_type.is_empty() {
        info.invoice_type = match (invoice_type.is_empty(), vat_type.is_empty()) {
            (false, false) => format!("{invoice_type}({vat_type})"),
            (false, true) => invoice_type,
            (true, false) => vat_type,
            (true, true) => String::new(),
        };
    }
    info.source = "xml".to_string();
    info.amount = if info.amount_tax > 0.0 {
        info.amount_tax
    } else {
        info.amount_no_tax
    };

    Ok(ParsedFormat { info })
}

fn local_name(name: &[u8]) -> String {
    let local = name.rsplit(|byte| *byte == b':').next().unwrap_or(name);
    String::from_utf8_lossy(local).into_owned()
}

fn parse_number(value: &str) -> f64 {
    value.replace(',', "").trim().parse().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_decimal_tax_rate_as_percent() {
        let parsed = parse_xml_content(
            "<EInvoice><TaxRate>0.13</TaxRate><TotalAmWithoutTax>100</TotalAmWithoutTax>\
             <TotalTaxAm>13</TotalTaxAm><TotalTax-includedAmount>113</TotalTax-includedAmount></EInvoice>",
            false,
        )
        .unwrap();

        assert_eq!(parsed.info.tax_rate, "13%");
    }
}
