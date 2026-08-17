pub(crate) fn normalize_text(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.replace("\r\n", "\n").replace('\r', "\n").chars() {
        let normalized = match ch {
            '０'..='９' => char::from_u32(ch as u32 - 0xfee0).unwrap_or(ch),
            'Ａ'..='Ｚ' | 'ａ'..='ｚ' => char::from_u32(ch as u32 - 0xfee0).unwrap_or(ch),
            '％' => '%',
            '．' => '.',
            '，' => ',',
            '：' => ':',
            '￥' => '¥',
            _ => ch,
        };
        output.push(normalized);
    }
    output
}

pub(crate) fn compact(input: &str) -> String {
    input.chars().filter(|ch| !ch.is_whitespace()).collect()
}

pub(crate) fn clean_name(input: &str) -> String {
    let mut value = input
        .trim_matches(|ch: char| ch.is_whitespace() || ":：,，。.、;；".contains(ch))
        .to_string();
    for label in [
        "统一社会信用代码",
        "纳税人识别号",
        "开户银行",
        "银行账号",
        "地址电话",
        "名称:",
        "名称：",
        "下载次数",
        "查验次数",
        "开具次数",
        "打印次数",
    ] {
        if let Some(index) = value.find(label) {
            value.truncate(index);
        }
    }
    for suffix in ["有限责任公司", "股份有限公司", "有限公司"] {
        let Some(index) = value.find(suffix) else {
            continue;
        };
        let end = index + suffix.len();
        let tail = &value[end..];
        if ["信用", "代码", "识别", "纳税"]
            .iter()
            .any(|label| tail.contains(label))
        {
            value.truncate(end);
        }
    }
    value = value
        .trim_matches(|ch: char| ch.is_whitespace() || ":：,，。.、;；".contains(ch))
        .to_string();
    if value.chars().count() < 2 || !value.chars().any(is_cjk) || is_rejected_name(&value) {
        return String::new();
    }
    value
}

pub(crate) fn normalize_credit_code(input: &str) -> String {
    input
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_uppercase())
        .collect()
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x3400..=0x9fff | 0xf900..=0xfaff)
}

fn is_rejected_name(value: &str) -> bool {
    [
        "购买方",
        "购买方信息",
        "销售方",
        "销售方信息",
        "名称",
        "信息",
        "纳税人",
        "地址",
        "电话",
        "开户行",
        "账号",
        "项目名称",
        "规格型号",
        "单位",
        "数量",
        "单价",
        "金额",
        "税率",
        "税额",
        "合计",
        "价税合计",
        "金额合计",
        "备注",
        "开票人",
        "收款人",
        "复核人",
        "电子发票",
        "增值税专用发票",
        "普通发票",
    ]
    .contains(&value)
        || value.contains("非税收入统一票据")
}
