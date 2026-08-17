mod amounts;
mod fields;
mod items;
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
    let mut amounts = amounts::extract_amounts(&normalized, identity.is_non_tax);

    let validation = validate_amounts(&mut amounts, source, identity.is_non_tax);
    let amount = if amounts.amount_tax > 0.0 {
        amounts.amount_tax
    } else {
        amounts.amount_no_tax
    };
    let line_items = items::extract_line_items(
        page,
        &normalized,
        amounts.amount_no_tax,
        amounts.tax_amount,
        &amounts.tax_rate,
    );

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
        line_items,
        is_ticket: identity.is_ticket,
        is_non_tax: identity.is_non_tax,
        amount_validation: validation,
        raw_text: include_raw_text.then_some(raw_text),
    }
}

fn validate_amounts(
    amounts: &mut amounts::Amounts,
    source: &str,
    is_non_tax: bool,
) -> Option<AmountValidation> {
    if is_non_tax || amounts.amount_tax <= 0.0 || amounts.amount_no_tax <= 0.0 {
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
            amounts.tax_rate = amounts::derive_tax_rate(no_tax, amounts.tax_amount);
            return None;
        }
    }
    if amounts.amount_no_tax > 0.0 && amounts.amount_no_tax < amounts.amount_tax {
        let tax = round_money(amounts.amount_tax - amounts.amount_no_tax);
        if valid_tax_rate(amounts.amount_no_tax, tax) {
            amounts.tax_amount = tax;
            amounts.tax_rate = amounts::derive_tax_rate(amounts.amount_no_tax, tax);
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
        assert_eq!(info.invoice_type, "vat-general");
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
    fn distinguishes_vat_kind_before_transport_usage() {
        let general = parse_recognition_page(
            &RecognitionPage::from_text(
                "电子发票（普通发票）\n旅客运输服务\n发票号码：26327000001246832832",
            ),
            0,
            "pdf-text",
            false,
        );
        let special = parse_recognition_page(
            &RecognitionPage::from_text(
                "电子发票（增值税专用发票）\n发票号码：25322000000337005189",
            ),
            0,
            "pdf-text",
            false,
        );

        assert_eq!(general.invoice_type, "vat-general");
        assert!(general.is_ticket);
        assert_eq!(special.invoice_type, "vat-special");
        assert!(!special.is_ticket);
    }

    #[test]
    fn extracts_invoice_clerk_from_the_block_after_its_label() {
        let page = RecognitionPage::from_text(
            "开票人:\n唐嫣\n成品油\n电子发票（普通发票）\n发票号码：26437000000252671049\n\
             开票日期：2026年08月07日\n价税合计（大写）叁佰伍拾圆整\n（小写）¥350.00",
        );

        let info = parse_recognition_page(&page, 0, "pdf-text", false);

        assert_eq!(info.invoice_clerk, "唐嫣");
    }

    #[test]
    fn extracts_invoice_clerk_after_repeated_labels() {
        let page = RecognitionPage::from_text(
            "电子发票（普通发票）\n价税合计（小写）¥1813.86\n\
             开票人：\t开票人：\t开票人：\t王梅",
        );

        let info = parse_recognition_page(&page, 0, "pdf-text", false);

        assert_eq!(info.invoice_clerk, "王梅");
    }

    #[test]
    fn text_order_repairs_mirrored_coordinates_in_tab_packed_pdf() {
        let raw_text = "长沙万漫网络科技有限公司\t安徽小菜园餐饮管理有限责任公司长沙岳麓王府井店\t*生产生活服务*餐饮服务\t杨甜梅\t26432000001576189981\t2026年07月07日\t杨甜梅\t91430104MA4T8FT50U\t91430104MAEP5LEU4X\t发票号码：\t开票日期：\t购\t买\t方\t信\t息\t统一社会信用代码/纳税人识别号：\t销\t售\t方\t信\t息\t统一社会信用代码/纳税人识别号：\t名称：\t名称：\t项目名称\t规格型号\t单  位\t数  量\t单  价\t金  额\t税率/征收率\t税  额\t合\t计\t价税合计（大写）\t（小写）\t备\t注\t开票人：\t6%\t¥\t405.66\t¥\t405.66\t405.66\t24.34\t24.34\t1\n肆佰叁拾圆整\t¥\t电子发票（普通发票）\t430.00";
        let page = RecognitionPage {
            text: raw_text.to_string(),
            lines: vec![crate::RecognitionLine {
                words: vec![
                    positioned_word("安徽小菜园餐饮管理有限责任公司长沙岳麓王府井店", 100.0),
                    positioned_word("长沙万漫网络科技有限公司", 700.0),
                    positioned_word("91430104MAEP5LEU4X", 100.0),
                    positioned_word("91430104MA4T8FT50U", 700.0),
                ],
                confidence: 1.0,
            }],
            img_w: 1000,
            img_h: 700,
        };

        let info = parse_recognition_page(&page, 0, "pdf-text", true);

        assert_eq!(info.invoice_no, "26432000001576189981");
        assert_eq!(info.invoice_date, "2026-07-07");
        assert_eq!(info.buyer_name, "长沙万漫网络科技有限公司");
        assert_eq!(info.buyer_credit_code, "91430104MA4T8FT50U");
        assert_eq!(
            info.seller_name,
            "安徽小菜园餐饮管理有限责任公司长沙岳麓王府井店"
        );
        assert_eq!(info.seller_credit_code, "91430104MAEP5LEU4X");
        assert_eq!(info.amount_tax, 430.0);
        assert_eq!(info.amount_no_tax, 405.66);
        assert_eq!(info.tax_amount, 24.34);
        assert_eq!(info.tax_rate, "6%");
        assert_eq!(info.amount_uppercase, "肆佰叁拾圆整");
        assert_eq!(info.invoice_clerk, "杨甜梅");
        assert_eq!(info.line_items.len(), 1);
        assert_eq!(info.line_items[0].project_name, "*生产生活服务*餐饮服务");
    }

    fn positioned_word(text: &str, x: f64) -> crate::RecognitionWord {
        crate::RecognitionWord {
            text: text.to_string(),
            x,
            ..Default::default()
        }
    }

    #[test]
    fn reconstructs_buyer_and_seller_from_cjk_character_columns() {
        let buyer_name = "长沙万漫网络科技有限公司";
        let seller_name = "杭州携华网络科技有限公司长沙分公司";
        let buyer_code = "91430104MA4T8FT50U";
        let seller_code = "91430112MA4QCKJQ1E";
        let authority_line = two_column_character_line("国家税务总局", "税务局");
        let name_line =
            two_column_character_line(&format!("息{buyer_name}售"), &format!("息{seller_name}"));
        let code_line = two_column_character_line(buyer_code, seller_code);
        let text = [
            "电子发票（普通发票）".to_string(),
            "发票号码：26437000000232291279".to_string(),
            "开票日期：2026年07月07日".to_string(),
            name_line
                .words
                .iter()
                .map(|word| word.text.as_str())
                .collect::<Vec<_>>()
                .join("\t"),
            code_line
                .words
                .iter()
                .map(|word| word.text.as_str())
                .collect::<Vec<_>>()
                .join("\t"),
            "价税合计（小写）¥43.33".to_string(),
        ]
        .join("\n");
        let page = RecognitionPage {
            text,
            lines: vec![authority_line, name_line, code_line],
            img_w: 800,
            img_h: 600,
        };

        let info = parse_recognition_page(&page, 0, "pdf-text", false);

        assert_eq!(info.buyer_name, buyer_name);
        assert_eq!(info.buyer_credit_code, buyer_code);
        assert_eq!(info.seller_name, seller_name);
        assert_eq!(info.seller_credit_code, seller_code);
    }

    #[test]
    fn splits_interleaved_party_names_without_coordinates() {
        let page = RecognitionPage::from_text(
            "电子发票（普通发票）\n旅客运输服务\n发票号码：26437000000232291279\n\
             买\t名称：\t长沙万漫网络科技有限公司\t售\t名称：\t杭州携华网络科技有限公司长沙分公司\n\
             *\t交\t通\t运\t输\t服\t务\t*\t客\t运\t服\t务\t费\t无\t次\t1\t4\t2\t.\t0\t7\t4\t2\t.\t0\t7\t3\t%\t1\t.\t2\t6\n\
             合计\t¥42.07\t¥1.26\n税率/征收率\t3\t%\n价税合计（小写）¥43.33",
        );

        let info = parse_recognition_page(&page, 0, "pdf-text", false);

        assert_eq!(info.buyer_name, "长沙万漫网络科技有限公司");
        assert_eq!(info.seller_name, "杭州携华网络科技有限公司长沙分公司");
        assert_eq!(info.tax_rate, "3%");
        assert_eq!(info.line_items.len(), 1);
        assert_eq!(info.line_items[0].project_name, "*交通运输服务*客运服务费");
        assert_eq!(info.line_items[0].amount, 42.07);
        assert_eq!(info.line_items[0].tax_amount, 1.26);
    }

    #[test]
    fn validates_discounted_fuel_invoice_from_net_totals() {
        let page = RecognitionPage::from_text(
            "长沙万漫网络科技有限公司\t中化石油 湖 南有限公司长沙东方红加油 站\t\
             *汽 油 *95#车用汽 油 (VIB)\t*汽 油 *95#车用汽 油 (VIB)\t95#\t向 智 玉\t\
             26432000001659042406\t2026年07月16日\t向智玉\t91430104MA4T8FT50U\t\
             91430100MA4L2W941K\t发票号码：\t开票日期：\t购\t买\t方\t信\t息\t\
             统一社会信用代码/纳税人识别号：\t销\t售\t方\t信\t息\t\
             统一社会信用代码/纳税人识别号：\t名称：\t名称：\t项目名称\t规格型号\t\
             单  位\t数  量\t单  价\t金  额\t税率/征收率\t税  额\t合\t计\t\
             价税合计（大写）\t（小写）\t备\t注\t开票人：\t升\t13%\t13%\t24.83801\t\
             8.19469\t¥\t¥\t203.54\t-10.99\t192.55\t26.46\t-1.43\t25.03\n\
             贰佰壹拾 柒圆伍角捌分\t¥\t电子发票（普通发票）\t217.58\n成 品 油",
        );

        let info = parse_recognition_page(&page, 0, "pdf-text", false);

        assert_eq!(info.amount_tax, 217.58);
        assert_eq!(info.amount_no_tax, 192.55);
        assert_eq!(info.tax_amount, 25.03);
        assert!(info.amount_validation.is_none());
        assert_eq!(info.line_items.len(), 1);
        assert_eq!(
            info.line_items[0].project_name,
            "*汽 油 *95#车用汽 油 (VIB)"
        );
        assert_eq!(info.line_items[0].amount, 192.55);
        assert_eq!(info.line_items[0].tax_amount, 25.03);
    }

    #[test]
    fn validates_transport_discount_from_net_totals() {
        let page = RecognitionPage::from_text(
            "电子发票（普通发票）\n旅客运输服务\n\
             *交通运输服务*客运服务费\t72.98\t3%\t2.19\n\
             *交通运输服务*客运服务费\t-17.18\t3%\t-0.52\n\
             合计\t¥55.80\t¥1.67\n价税合计（小写）¥57.47",
        );

        let info = parse_recognition_page(&page, 0, "pdf-text", false);

        assert!(info.is_ticket);
        assert_eq!(info.amount_tax, 57.47);
        assert_eq!(info.amount_no_tax, 55.8);
        assert_eq!(info.tax_amount, 1.67);
        assert_eq!(info.tax_rate, "3%");
        assert!(info.amount_validation.is_none());
    }

    #[test]
    fn accepts_individual_business_store_as_seller() {
        let page = RecognitionPage::from_text(
            "长沙万漫网络科技有限公司\t湖南湘江新区溪山烟雨餐饮店（个体工商户）\t\
             91430104MA4T8FT50U\t92430104MAKGPDD7XK\t购买方信息\t销售方信息\n\
             发票号码：26432000001575516481\n开票日期：2026年07月07日\n\
             价税合计（小写）¥500.00",
        );

        let info = parse_recognition_page(&page, 0, "pdf-text", false);

        assert_eq!(info.buyer_name, "长沙万漫网络科技有限公司");
        assert_eq!(info.buyer_credit_code, "91430104MA4T8FT50U");
        assert_eq!(info.seller_name, "湖南湘江新区溪山烟雨餐饮店（个体工商户）");
        assert_eq!(info.seller_credit_code, "92430104MAKGPDD7XK");
    }

    fn two_column_character_line(left: &str, right: &str) -> crate::RecognitionLine {
        let mut words = left
            .chars()
            .enumerate()
            .map(|(index, character)| {
                positioned_word(&character.to_string(), 20.0 + index as f64 * 8.0)
            })
            .collect::<Vec<_>>();
        words.extend(right.chars().enumerate().map(|(index, character)| {
            positioned_word(&character.to_string(), 430.0 + index as f64 * 8.0)
        }));
        crate::RecognitionLine {
            words,
            confidence: 1.0,
        }
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
