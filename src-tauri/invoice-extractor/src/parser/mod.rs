mod amounts;
mod fields;
mod normalize;

use crate::{AmountValidation, InvoiceInfo, RecognitionPage};
pub(crate) use amounts::normalize_tax_rate;

pub(crate) fn parse_recognition_page(
    page: &RecognitionPage,
    page_index: u32,
    source: &str,
    include_raw_text: bool,
) -> InvoiceInfo {
    let raw_text = if page.text.trim().is_empty() {
        page.lines
            .iter()
            .map(|line| {
                line.words
                    .iter()
                    .map(|word| word.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        page.text.clone()
    };
    let normalized = normalize::normalize_text(&raw_text);
    let identity = fields::extract_identity(&normalized, page);
    let mut amounts =
        amounts::extract_amounts(&normalized, identity.is_ticket, identity.is_non_tax);

    let validation = validate_amounts(
        &mut amounts,
        source,
        identity.is_ticket,
        identity.is_non_tax,
    );
    let amount = if amounts.amount_tax > 0.0 {
        amounts.amount_tax
    } else {
        amounts.amount_no_tax
    };

    InvoiceInfo {
        page_index,
        source: source.to_string(),
        invoice_no: identity.invoice_no,
        invoice_date: identity.invoice_date,
        invoice_type: identity.invoice_type,
        buyer_name: identity.buyer_name,
        buyer_credit_code: identity.buyer_credit_code,
        seller_name: identity.seller_name,
        seller_credit_code: identity.seller_credit_code,
        amount,
        amount_tax: amounts.amount_tax,
        amount_no_tax: amounts.amount_no_tax,
        tax_amount: amounts.tax_amount,
        tax_rate: amounts.tax_rate,
        amount_uppercase: amounts.amount_uppercase,
        invoice_clerk: identity.invoice_clerk,
        is_ticket: identity.is_ticket,
        is_non_tax: identity.is_non_tax,
        amount_validation: validation,
        raw_text: include_raw_text.then_some(raw_text),
    }
}

fn validate_amounts(
    amounts: &mut amounts::Amounts,
    source: &str,
    is_ticket: bool,
    is_non_tax: bool,
) -> Option<AmountValidation> {
    if is_ticket || is_non_tax || amounts.amount_tax <= 0.0 || amounts.amount_no_tax <= 0.0 {
        return None;
    }
    let sum = round_money(amounts.amount_no_tax + amounts.tax_amount);
    if (sum - amounts.amount_tax).abs() <= 0.02 {
        return None;
    }

    let original = AmountValidation {
        amount_tax: amounts.amount_tax,
        amount_no_tax: amounts.amount_no_tax,
        tax_amount: amounts.tax_amount,
        source: source.to_string(),
    };
    if amounts.tax_amount > 0.0 && amounts.tax_amount < amounts.amount_tax {
        let no_tax = round_money(amounts.amount_tax - amounts.tax_amount);
        if valid_tax_rate(no_tax, amounts.tax_amount) {
            amounts.amount_no_tax = no_tax;
            return None;
        }
    }
    if amounts.amount_no_tax > 0.0 && amounts.amount_no_tax < amounts.amount_tax {
        let tax = round_money(amounts.amount_tax - amounts.amount_no_tax);
        if valid_tax_rate(amounts.amount_no_tax, tax) {
            amounts.tax_amount = tax;
            return None;
        }
    }
    Some(original)
}

pub(crate) fn round_money(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

pub(crate) fn valid_tax_rate(amount_no_tax: f64, tax_amount: f64) -> bool {
    if amount_no_tax <= 0.0 || tax_amount < 0.0 {
        return false;
    }
    let rate = tax_amount / amount_no_tax;
    [0.0, 0.01, 0.03, 0.05, 0.06, 0.09, 0.13]
        .iter()
        .any(|expected| (rate - expected).abs() < 0.005)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_standard_vat_invoice() {
        let page = RecognitionPage::from_text(
            "电子发票（普通发票）\n发票号码：25322000000337005189\n开票日期：2025年07月22日\n\
             购买方名称：江苏测试科技有限公司\n统一社会信用代码：9132020013590404XW\n\
             销售方名称：无锡示例商贸有限公司\n统一社会信用代码：91320200796148368W\n\
             合计 ¥100.00 ¥13.00\n税率 13%\n价税合计（大写）壹佰壹拾叁圆整\n\
             价税合计（小写）¥113.00\n开票人：张三",
        );
        let info = parse_recognition_page(&page, 0, "pdf-text", true);
        assert_eq!(info.invoice_no, "25322000000337005189");
        assert_eq!(info.invoice_date, "2025-07-22");
        assert_eq!(info.buyer_name, "江苏测试科技有限公司");
        assert_eq!(info.seller_name, "无锡示例商贸有限公司");
        assert_eq!(info.amount_tax, 113.0);
        assert_eq!(info.amount_no_tax, 100.0);
        assert_eq!(info.tax_amount, 13.0);
        assert_eq!(info.tax_rate, "13%");
        assert_eq!(info.amount_uppercase, "壹佰壹拾叁圆整");
        assert_eq!(info.invoice_clerk, "张三");
        assert!(info.amount_validation.is_none());
    }

    #[test]
    fn extracts_non_tax_invoice() {
        let page = RecognitionPage::from_text(
            "江苏省非税收入统一票据（电子）\n票据号码：32000123456789012345\n\
             开票日期：2026-04-28\n交款人：张三\n收款单位：某某行政服务中心\n金额合计（小写）¥88.00"
        );
        let info = parse_recognition_page(&page, 0, "pdf-text", false);
        assert!(info.is_non_tax);
        assert_eq!(info.invoice_no, "32000123456789012345");
        assert_eq!(info.buyer_name, "张三");
        assert_eq!(info.seller_name, "某某行政服务中心");
        assert_eq!(info.amount_tax, 88.0);
        assert_eq!(info.amount_no_tax, 88.0);
    }

    #[test]
    fn extracts_values_from_separate_pdf_text_blocks() {
        let page = RecognitionPage::from_text(
            "电子发票（普通发票）\n发票号码：\n开票日期：\n购买方信息\n销售方信息\n\
             名称：\n名称：\n26432000001910446111\n2026年08月16日\n\
             长沙百寻网络科技有限公司\n91430104MACJBWXN1K\n\
             植觉素茶餐（长沙）有限公司\n91430104MAD21GYF0P\n\
             合计 ¥335.64 ¥3.36\n税率\n1%\n价税合计（小写）¥339.00",
        );

        let info = parse_recognition_page(&page, 0, "pdf-text", false);
        assert_eq!(info.invoice_no, "26432000001910446111");
        assert_eq!(info.invoice_date, "2026-08-16");
        assert_eq!(info.buyer_name, "长沙百寻网络科技有限公司");
        assert_eq!(info.buyer_credit_code, "91430104MACJBWXN1K");
        assert_eq!(info.seller_name, "植觉素茶餐（长沙）有限公司");
        assert_eq!(info.seller_credit_code, "91430104MAD21GYF0P");
        assert_eq!(info.tax_rate, "1%");
    }

    #[test]
    fn maps_unqualified_ocr_name_lines_before_table_headers() {
        let page = RecognitionPage {
            text: "电子发票（普通发票）\n名称：长沙百寻络科技有限公司\n\
                   名称:长沙熙之棠餐饮管理有限公司\n\
                   统一社会信用代码/纳税人识别号：91430104MACJBWXN1K\n\
                   统一社会信用代码/纳税人识别号：91430104MA4QLJ16X6\n\
                   税额\n单位\n数量\n单价\n项目名称\n金额\n6%"
                .to_string(),
            lines: vec![
                crate::RecognitionLine {
                    words: vec![crate::RecognitionWord {
                        text: "名称：长沙百寻络科技有限公司".to_string(),
                        x: 20.0,
                        y: 100.0,
                        w: 300.0,
                        h: 20.0,
                    }],
                    confidence: 0.9,
                },
                crate::RecognitionLine {
                    words: vec![crate::RecognitionWord {
                        text: "单位".to_string(),
                        x: 100.0,
                        y: 130.0,
                        w: 40.0,
                        h: 20.0,
                    }],
                    confidence: 0.9,
                },
            ],
            img_w: 1000,
            img_h: 700,
        };

        let info = parse_recognition_page(&page, 0, "ocr", false);

        assert_eq!(info.buyer_name, "长沙百寻络科技有限公司");
        assert_eq!(info.seller_name, "长沙熙之棠餐饮管理有限公司");
    }

    #[test]
    fn maps_form_values_to_buyer_and_seller_by_horizontal_position() {
        let values = [
            ("长沙京东厚成贸易有限公司", 350.0),
            ("长沙万漫网络科技有限公司", 50.0),
            ("91430112MA4PQ4A2XY", 350.0),
            ("91430104MA4T8FT50U", 50.0),
        ];
        let page = RecognitionPage {
            text: values
                .iter()
                .map(|(value, _)| *value)
                .collect::<Vec<_>>()
                .join("\n"),
            lines: values
                .into_iter()
                .map(|(value, x)| crate::RecognitionLine {
                    words: vec![crate::RecognitionWord {
                        text: value.to_string(),
                        x,
                        ..Default::default()
                    }],
                    confidence: 1.0,
                })
                .collect(),
            img_w: 600,
            img_h: 800,
        };

        let info = parse_recognition_page(&page, 0, "pdf-text", false);

        assert_eq!(info.buyer_name, "长沙万漫网络科技有限公司");
        assert_eq!(info.buyer_credit_code, "91430104MA4T8FT50U");
        assert_eq!(info.seller_name, "长沙京东厚成贸易有限公司");
        assert_eq!(info.seller_credit_code, "91430112MA4PQ4A2XY");
    }

    #[test]
    fn derives_tax_rate_when_text_does_not_include_it() {
        let page = RecognitionPage::from_text(
            "电子发票（普通发票）\n合计 ¥100.00 ¥13.00\n价税合计（小写）¥113.00",
        );

        let info = parse_recognition_page(&page, 0, "pdf-text", false);

        assert_eq!(info.tax_rate, "13%");
    }

    #[test]
    fn does_not_invent_zero_rate_without_tax_evidence() {
        let page = RecognitionPage::from_text("电子发票（普通发票）\n价税合计（小写）¥247.00");

        let info = parse_recognition_page(&page, 0, "pdf-text", false);

        assert!(info.tax_rate.is_empty());
    }

    #[test]
    fn preserves_multiple_tax_rates_in_document_order() {
        let page = RecognitionPage::from_text(
            "电子发票（普通发票）\n项目一 13%\n项目二 免税\n项目三 1%\n价税合计（小写）¥113.00",
        );

        let info = parse_recognition_page(&page, 0, "pdf-text", false);

        assert_eq!(info.tax_rate, "13%,免税,1%");
    }
}
