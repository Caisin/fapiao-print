use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtractionOptions {
    pub use_ocr: bool,
    pub ocr_precision: String,
    pub include_raw_text: bool,
}

impl Default for ExtractionOptions {
    fn default() -> Self {
        Self {
            use_ocr: true,
            ocr_precision: "standard".to_string(),
            include_raw_text: true,
        }
    }
}

impl ExtractionOptions {
    pub(crate) fn normalize(mut self) -> Self {
        if !matches!(self.ocr_precision.as_str(), "fast" | "standard" | "precise") {
            self.ocr_precision = "standard".to_string();
        }
        self
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecognitionWord {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecognitionLine {
    pub words: Vec<RecognitionWord>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecognitionPage {
    pub text: String,
    pub lines: Vec<RecognitionLine>,
    pub img_w: u32,
    pub img_h: u32,
}

impl RecognitionPage {
    pub fn from_text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    pub fn has_content(&self) -> bool {
        !self.text.trim().is_empty() || self.lines.iter().any(|line| !line.words.is_empty())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AmountValidation {
    pub amount_tax: f64,
    pub amount_no_tax: f64,
    pub tax_amount: f64,
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceInfo {
    pub page_index: u32,
    pub source: String,
    pub invoice_no: String,
    pub invoice_date: String,
    pub invoice_type: String,
    pub buyer_name: String,
    pub buyer_credit_code: String,
    pub seller_name: String,
    pub seller_credit_code: String,
    pub amount: f64,
    pub amount_tax: f64,
    pub amount_no_tax: f64,
    pub tax_amount: f64,
    pub tax_rate: String,
    pub amount_uppercase: String,
    pub invoice_clerk: String,
    pub is_ticket: bool,
    pub is_non_tax: bool,
    pub amount_validation: Option<AmountValidation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_text: Option<String>,
}

impl InvoiceInfo {
    pub fn has_useful_data(&self) -> bool {
        !self.invoice_no.is_empty()
            || !self.invoice_date.is_empty()
            || !self.seller_name.is_empty()
            || !self.buyer_name.is_empty()
            || self.amount_tax > 0.0
            || self.amount_no_tax > 0.0
    }

    pub(crate) fn has_core_data(&self) -> bool {
        !self.seller_name.is_empty() && (self.amount_tax > 0.0 || self.amount_no_tax > 0.0)
    }

    pub(crate) fn merge_missing(&mut self, other: InvoiceInfo) {
        macro_rules! fill_string {
            ($field:ident) => {
                if self.$field.is_empty() {
                    self.$field = other.$field;
                }
            };
        }
        fill_string!(invoice_no);
        fill_string!(invoice_date);
        fill_string!(buyer_name);
        fill_string!(buyer_credit_code);
        fill_string!(seller_name);
        fill_string!(seller_credit_code);
        fill_string!(tax_rate);
        fill_string!(amount_uppercase);
        fill_string!(invoice_clerk);
        if self.invoice_type.is_empty() || self.invoice_type == "unknown" {
            self.invoice_type = other.invoice_type;
        }
        if self.amount_tax <= 0.0 {
            self.amount_tax = other.amount_tax;
        }
        if self.amount_no_tax <= 0.0 {
            self.amount_no_tax = other.amount_no_tax;
        }
        if self.tax_amount <= 0.0 {
            self.tax_amount = other.tax_amount;
        }
        if self.amount <= 0.0 {
            self.amount = other.amount;
        }
        self.is_ticket |= other.is_ticket;
        self.is_non_tax |= other.is_non_tax;
        if self.amount_validation.is_none() {
            self.amount_validation = other.amount_validation;
        }
        if self.raw_text.as_ref().map_or(true, String::is_empty) {
            self.raw_text = other.raw_text;
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceFileResult {
    pub success: bool,
    pub file_path: String,
    pub file_name: String,
    pub file_type: String,
    pub page_count: u32,
    pub invoices: Vec<InvoiceInfo>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryExtractionError {
    pub file_path: String,
    pub error: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceDirectoryResult {
    pub success: bool,
    pub directory_path: String,
    pub matched_file_count: usize,
    pub extracted_file_count: usize,
    pub failed_file_count: usize,
    pub files: Vec<InvoiceFileResult>,
    pub errors: Vec<DirectoryExtractionError>,
}
