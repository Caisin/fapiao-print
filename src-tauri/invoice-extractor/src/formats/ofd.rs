use super::xml::parse_xml_content;
use super::ParsedFormat;
use crate::parser::parse_recognition_page;
use crate::RecognitionPage;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

const MAX_XML_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TOTAL_XML_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 4096;

pub(crate) fn read_ofd(path: &Path, include_raw_text: bool) -> Result<ParsedFormat, String> {
    let file = File::open(path).map_err(|error| format!("打开 OFD 文件失败: {error}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|error| format!("OFD 压缩包无效: {error}"))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err("OFD 文件包含过多条目".to_string());
    }
    let mut all_text = Vec::new();
    let mut structured = crate::InvoiceInfo::default();
    let mut object_text = HashMap::<u32, String>::new();
    let mut tag_refs = HashMap::<String, Vec<u32>>::new();
    let mut total_xml_bytes = 0_u64;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("读取 OFD 条目失败: {error}"))?;
        if !entry.name().to_ascii_lowercase().ends_with(".xml") || entry.size() > MAX_XML_BYTES {
            continue;
        }
        total_xml_bytes = total_xml_bytes.saturating_add(entry.size());
        if total_xml_bytes > MAX_TOTAL_XML_BYTES {
            return Err("OFD XML 数据超过安全限制".to_string());
        }
        let mut xml = String::new();
        entry
            .read_to_string(&mut xml)
            .map_err(|error| format!("读取 OFD XML 失败: {error}"))?;
        if let Ok(parsed) = parse_xml_content(&xml, false) {
            structured.merge_missing(parsed.info);
        }
        structured.merge_missing(parse_custom_data(&xml)?);
        collect_text_objects(&xml, &mut object_text)?;
        collect_tag_refs(&xml, &mut tag_refs)?;
        all_text.extend(extract_visible_text(&xml)?);
    }

    if all_text.is_empty() {
        return Err("OFD 中未找到可解析的 XML 文本".to_string());
    }
    let text = all_text.join("\n");
    merge_tagged_fields(&mut structured, &tag_refs, &object_text);
    let parsed = parse_recognition_page(
        &RecognitionPage::from_text(text.clone()),
        0,
        "ofd",
        include_raw_text,
    );
    structured.merge_missing(parsed);
    structured.source = "ofd".to_string();
    structured.amount = if structured.amount_tax > 0.0 {
        structured.amount_tax
    } else {
        structured.amount_no_tax
    };
    if include_raw_text {
        structured.raw_text = Some(text.clone());
    }

    Ok(ParsedFormat { info: structured })
}

fn parse_custom_data(xml: &str) -> Result<crate::InvoiceInfo, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut current_name = None::<String>;
    let mut values = HashMap::<String, String>::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) if local_name(event.name().as_ref()) == "CustomData" => {
                current_name = attribute_value(&event, b"Name");
            }
            Ok(Event::Text(event)) if current_name.is_some() => {
                let value = event
                    .unescape()
                    .map_err(|error| format!("OFD CustomData 解码失败: {error}"))?
                    .trim()
                    .to_string();
                if let Some(name) = current_name.take() {
                    if !value.is_empty() {
                        values.insert(name, value);
                    }
                }
            }
            Ok(Event::End(event)) if local_name(event.name().as_ref()) == "CustomData" => {
                current_name = None;
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(format!("OFD CustomData 解析失败: {error}")),
            _ => {}
        }
    }

    let mut info = crate::InvoiceInfo {
        invoice_no: take_alias(&values, &["发票号码", "InvoiceNo"]),
        invoice_date: take_alias(&values, &["开票日期", "IssueDate"]),
        buyer_name: take_alias(&values, &["购买方名称", "BuyerName"]),
        buyer_credit_code: take_alias(&values, &["购买方纳税人识别号", "BuyerTaxID"]),
        seller_name: take_alias(&values, &["销售方名称", "SellerName"]),
        seller_credit_code: take_alias(&values, &["销售方纳税人识别号", "SellerTaxID"]),
        amount_no_tax: parse_alias_number(&values, &["合计金额", "TaxExclusiveTotalAmount"]),
        tax_amount: parse_alias_number(&values, &["合计税额", "TaxTotalAmount"]),
        amount_tax: parse_alias_number(&values, &["价税合计", "TaxInclusiveTotalAmount", "Amount"]),
        ..Default::default()
    };
    if info.amount_tax <= 0.0 && info.amount_no_tax > 0.0 {
        info.amount_tax = crate::parser::round_money(info.amount_no_tax + info.tax_amount);
    }
    Ok(info)
}

fn collect_text_objects(xml: &str, output: &mut HashMap<u32, String>) -> Result<(), String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut current_id = None::<u32>;
    let mut in_text_code = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) if local_name(event.name().as_ref()) == "TextObject" => {
                current_id = attribute_value(&event, b"ID").and_then(|value| value.parse().ok());
            }
            Ok(Event::Start(event)) if local_name(event.name().as_ref()) == "TextCode" => {
                in_text_code = true;
            }
            Ok(Event::Text(event)) if in_text_code && current_id.is_some() => {
                let value = event
                    .unescape()
                    .map_err(|error| format!("OFD TextCode 解码失败: {error}"))?;
                output
                    .entry(current_id.unwrap())
                    .or_default()
                    .push_str(value.trim());
            }
            Ok(Event::End(event)) => match local_name(event.name().as_ref()).as_str() {
                "TextCode" => in_text_code = false,
                "TextObject" => current_id = None,
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(error) => return Err(format!("OFD TextObject 解析失败: {error}")),
            _ => {}
        }
    }
    Ok(())
}

fn collect_tag_refs(xml: &str, output: &mut HashMap<String, Vec<u32>>) -> Result<(), String> {
    const FIELDS: &[&str] = &[
        "InvoiceNo",
        "IssueDate",
        "BuyerName",
        "BuyerTaxID",
        "SellerName",
        "SellerTaxID",
        "TaxExclusiveTotalAmount",
        "TaxTotalAmount",
        "TaxInclusiveTotalAmount",
        "Amount",
    ];
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut current_field = None::<String>;
    let mut in_object_ref = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                let tag = local_name(event.name().as_ref());
                if FIELDS.contains(&tag.as_str()) {
                    current_field = Some(tag);
                } else if tag == "ObjectRef" {
                    in_object_ref = true;
                }
            }
            Ok(Event::Text(event)) if in_object_ref && current_field.is_some() => {
                let value = event
                    .unescape()
                    .map_err(|error| format!("OFD ObjectRef 解码失败: {error}"))?;
                if let Ok(id) = value.trim().parse::<u32>() {
                    output
                        .entry(current_field.clone().unwrap())
                        .or_default()
                        .push(id);
                }
            }
            Ok(Event::End(event)) => {
                let tag = local_name(event.name().as_ref());
                if tag == "ObjectRef" {
                    in_object_ref = false;
                }
                if FIELDS.contains(&tag.as_str()) {
                    current_field = None;
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(format!("OFD CustomTag 解析失败: {error}")),
            _ => {}
        }
    }
    Ok(())
}

fn merge_tagged_fields(
    info: &mut crate::InvoiceInfo,
    refs: &HashMap<String, Vec<u32>>,
    object_text: &HashMap<u32, String>,
) {
    let get = |field: &str| -> String {
        refs.get(field)
            .and_then(|ids| ids.iter().find_map(|id| object_text.get(id)))
            .cloned()
            .unwrap_or_default()
    };
    let tagged = crate::InvoiceInfo {
        invoice_no: get("InvoiceNo"),
        invoice_date: get("IssueDate"),
        buyer_name: get("BuyerName"),
        buyer_credit_code: get("BuyerTaxID"),
        seller_name: get("SellerName"),
        seller_credit_code: get("SellerTaxID"),
        amount_no_tax: parse_number(&get("TaxExclusiveTotalAmount")),
        tax_amount: parse_number(&get("TaxTotalAmount")),
        amount_tax: parse_number(&get("TaxInclusiveTotalAmount")).max(parse_number(&get("Amount"))),
        ..Default::default()
    };
    info.merge_missing(tagged);
}

fn attribute_value(event: &quick_xml::events::BytesStart<'_>, expected: &[u8]) -> Option<String> {
    event.attributes().flatten().find_map(|attribute| {
        (local_name(attribute.key.as_ref()).as_bytes() == expected)
            .then(|| String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
    })
}

fn local_name(name: &[u8]) -> String {
    let local = name.rsplit(|byte| *byte == b':').next().unwrap_or(name);
    String::from_utf8_lossy(local).into_owned()
}

fn take_alias(values: &HashMap<String, String>, aliases: &[&str]) -> String {
    aliases
        .iter()
        .find_map(|alias| values.get(*alias))
        .cloned()
        .unwrap_or_default()
}

fn parse_alias_number(values: &HashMap<String, String>, aliases: &[&str]) -> f64 {
    parse_number(&take_alias(values, aliases))
}

fn parse_number(value: &str) -> f64 {
    value
        .trim()
        .trim_start_matches(['¥', '￥'])
        .replace(',', "")
        .parse()
        .unwrap_or(0.0)
}

fn extract_visible_text(xml: &str) -> Result<Vec<String>, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut values = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Text(event)) => {
                let value = event
                    .unescape()
                    .map_err(|error| format!("OFD XML 文本解码失败: {error}"))?;
                let value = value.trim();
                if !value.is_empty() {
                    values.push(value.to_string());
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(format!("OFD XML 解析失败: {error}")),
            _ => {}
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_custom_data_without_invoice_engine() {
        let xml = r#"<ofd:OFD xmlns:ofd="urn:ofd">
          <ofd:CustomData Name="发票号码">25322000000337005189</ofd:CustomData>
          <ofd:CustomData Name="购买方名称">江苏测试科技有限公司</ofd:CustomData>
          <ofd:CustomData Name="销售方名称">无锡示例商贸有限公司</ofd:CustomData>
          <ofd:CustomData Name="合计金额">100.00</ofd:CustomData>
          <ofd:CustomData Name="合计税额">13.00</ofd:CustomData>
        </ofd:OFD>"#;
        let info = parse_custom_data(xml).unwrap();
        assert_eq!(info.invoice_no, "25322000000337005189");
        assert_eq!(info.buyer_name, "江苏测试科技有限公司");
        assert_eq!(info.seller_name, "无锡示例商贸有限公司");
        assert_eq!(info.amount_tax, 113.0);
    }

    #[test]
    fn resolves_custom_tag_object_references() {
        let content = r#"<ofd:Page xmlns:ofd="urn:ofd">
          <ofd:TextObject ID="42"><ofd:TextCode>无锡示例商贸有限公司</ofd:TextCode></ofd:TextObject>
        </ofd:Page>"#;
        let tags = r#"<ofd:Tags xmlns:ofd="urn:ofd">
          <ofd:SellerName><ofd:ObjectRef>42</ofd:ObjectRef></ofd:SellerName>
        </ofd:Tags>"#;
        let mut objects = HashMap::new();
        let mut refs = HashMap::new();
        collect_text_objects(content, &mut objects).unwrap();
        collect_tag_refs(tags, &mut refs).unwrap();
        let mut info = crate::InvoiceInfo::default();
        merge_tagged_fields(&mut info, &refs, &objects);
        assert_eq!(info.seller_name, "无锡示例商贸有限公司");
    }
}
