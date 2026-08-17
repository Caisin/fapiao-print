use crate::backend::{NoOcr, OcrBackend};
use crate::formats::{read_ofd, read_pdf_pages, read_xml};
use crate::model::{
    DirectoryExtractionError, ExtractionOptions, InvoiceDirectoryResult, InvoiceFileResult,
    InvoiceInfo,
};
use crate::parser::parse_recognition_page;
use std::path::{Path, PathBuf};

pub struct InvoiceExtractor<B> {
    ocr: B,
}

impl<B: OcrBackend> InvoiceExtractor<B> {
    pub fn new(ocr: B) -> Self {
        Self { ocr }
    }

    pub fn extract_file(
        &self,
        file_path: impl AsRef<Path>,
        options: ExtractionOptions,
    ) -> Result<InvoiceFileResult, String> {
        let path = file_path.as_ref();
        if !path.is_file() {
            return Err(format!("发票文件不存在: {}", path.display()));
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "发票文件名不是有效 UTF-8".to_string())?
            .to_string();
        let file_type = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let options = options.normalize();
        let mut warnings = Vec::new();

        let invoices = match file_type.as_str() {
            "pdf" => self.extract_pdf(path, &options, &mut warnings)?,
            "ofd" => vec![read_ofd(path, options.include_raw_text)?.info],
            "xml" => vec![read_xml(path, options.include_raw_text)?.info],
            "jpg" | "jpeg" | "png" | "bmp" | "webp" | "tif" | "tiff" => {
                vec![self.extract_image(path, &options)?]
            }
            _ => {
                return Err(format!(
                    "不支持的发票文件格式: {}",
                    if file_type.is_empty() {
                        "无扩展名"
                    } else {
                        &file_type
                    }
                ))
            }
        };
        let page_count =
            u32::try_from(invoices.len()).map_err(|_| "发票页数超过支持范围".to_string())?;
        Ok(InvoiceFileResult {
            success: invoices.iter().any(InvoiceInfo::has_useful_data),
            file_path: path.to_string_lossy().into_owned(),
            file_name,
            file_type,
            page_count,
            invoices,
            warnings,
        })
    }

    pub fn extract_directory(
        &self,
        directory_path: impl AsRef<Path>,
        options: ExtractionOptions,
    ) -> Result<InvoiceDirectoryResult, String> {
        let directory = directory_path.as_ref();
        if !directory.is_dir() {
            return Err(format!("发票目录不存在: {}", directory.display()));
        }

        let mut paths = Vec::new();
        let mut errors = Vec::new();
        collect_invoice_paths(directory, &mut paths, &mut errors);
        paths.sort();

        let matched_file_count = paths.len();
        let mut files = Vec::with_capacity(matched_file_count);
        for path in paths {
            match self.extract_file(&path, options.clone()) {
                Ok(result) => files.push(result),
                Err(error) => errors.push(DirectoryExtractionError {
                    file_path: path.to_string_lossy().into_owned(),
                    error,
                }),
            }
        }
        let extracted_file_count = files.len();
        let failed_file_count = errors.len();
        let success = matched_file_count > 0
            && failed_file_count == 0
            && files.iter().all(|file| file.success);

        Ok(InvoiceDirectoryResult {
            success,
            directory_path: directory.to_string_lossy().into_owned(),
            matched_file_count,
            extracted_file_count,
            failed_file_count,
            files,
            errors,
        })
    }

    fn extract_pdf(
        &self,
        path: &Path,
        options: &ExtractionOptions,
        warnings: &mut Vec<String>,
    ) -> Result<Vec<InvoiceInfo>, String> {
        let pdf = read_pdf_pages(path)?;
        warnings.extend(pdf.warnings);
        if pdf.pages.is_empty() {
            return Err("PDF 不包含页面".to_string());
        }
        let ocr_available = options.use_ocr && self.ocr.is_available();
        let mut invoices = Vec::with_capacity(pdf.pages.len());

        for (page_index, page) in pdf.pages.into_iter().enumerate() {
            let page_index =
                u32::try_from(page_index).map_err(|_| "PDF 页码超过支持范围".to_string())?;
            let mut info = parse_recognition_page(
                &page,
                page_index,
                if page.has_content() {
                    "pdf-text"
                } else {
                    "none"
                },
                options.include_raw_text,
            );
            if ocr_available && !info.has_core_data() {
                match self
                    .ocr
                    .recognize_pdf_page(path, page_index, &options.ocr_precision)
                {
                    Ok(ocr_page) => {
                        let ocr_info = parse_recognition_page(
                            &ocr_page,
                            page_index,
                            "ocr",
                            options.include_raw_text,
                        );
                        let had_text = page.has_content();
                        info.merge_missing(ocr_info);
                        info.source = if had_text { "pdf-text+ocr" } else { "ocr" }.to_string();
                    }
                    Err(error) => {
                        warnings.push(format!("第 {} 页 OCR 失败: {error}", page_index + 1))
                    }
                }
            } else if options.use_ocr && !self.ocr.is_available() && !info.has_useful_data() {
                warnings.push(format!(
                    "第 {} 页没有可用文字层，且当前版本未配置 OCR",
                    page_index + 1
                ));
            }
            if info.invoice_no.is_empty() {
                info.invoice_no = invoice_number_from_file_name(path).unwrap_or_default();
            }
            invoices.push(info);
        }
        Ok(invoices)
    }

    fn extract_image(
        &self,
        path: &Path,
        options: &ExtractionOptions,
    ) -> Result<InvoiceInfo, String> {
        if !options.use_ocr {
            return Err("图片发票只能通过 OCR 提取，请启用 useOcr".to_string());
        }
        if !self.ocr.is_available() {
            return Err("图片发票需要配置 OCR 后端".to_string());
        }
        let page = self.ocr.recognize_image(path, &options.ocr_precision)?;
        Ok(parse_recognition_page(
            &page,
            0,
            "ocr",
            options.include_raw_text,
        ))
    }
}

fn invoice_number_from_file_name(path: &Path) -> Option<String> {
    let file_stem = path.file_stem()?.to_str()?;
    let mut digits = String::new();
    let mut candidates = Vec::new();
    for character in file_stem.chars().chain(std::iter::once('_')) {
        if character.is_ascii_digit() {
            digits.push(character);
        } else {
            if (8..=20).contains(&digits.len()) {
                candidates.push(std::mem::take(&mut digits));
            }
            digits.clear();
        }
    }
    candidates.into_iter().max_by_key(String::len)
}

fn collect_invoice_paths(
    directory: &Path,
    paths: &mut Vec<PathBuf>,
    errors: &mut Vec<DirectoryExtractionError>,
) {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(DirectoryExtractionError {
                file_path: directory.to_string_lossy().into_owned(),
                error: format!("读取目录失败: {error}"),
            });
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                errors.push(DirectoryExtractionError {
                    file_path: directory.to_string_lossy().into_owned(),
                    error: format!("读取目录项失败: {error}"),
                });
                continue;
            }
        };
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                errors.push(DirectoryExtractionError {
                    file_path: entry.path().to_string_lossy().into_owned(),
                    error: format!("读取文件类型失败: {error}"),
                });
                continue;
            }
        };
        if file_type.is_dir() {
            collect_invoice_paths(&entry.path(), paths, errors);
        } else if file_type.is_file() && is_supported_invoice_path(&entry.path()) {
            paths.push(entry.path());
        }
    }
}

fn is_supported_invoice_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("pdf" | "ofd" | "xml" | "jpg" | "jpeg" | "png" | "bmp" | "webp" | "tif" | "tiff")
    )
}

pub fn extract_file(file_path: impl AsRef<Path>) -> Result<InvoiceFileResult, String> {
    InvoiceExtractor::new(NoOcr).extract_file(file_path, ExtractionOptions::default())
}

pub fn extract_directory(
    directory_path: impl AsRef<Path>,
) -> Result<InvoiceDirectoryResult, String> {
    InvoiceExtractor::new(NoOcr).extract_directory(directory_path, ExtractionOptions::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn extracts_structured_xml_with_path_only_api() {
        let path = std::env::temp_dir().join(format!(
            "invoice-extractor-{}-{}.xml",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, r#"<EInvoice>
          <SellerInformation><SellerIdNum>91320200796148368W</SellerIdNum><SellerName>无锡示例商贸有限公司</SellerName></SellerInformation>
          <BuyerInformation><BuyerIdNum>9132020013590404XW</BuyerIdNum><BuyerName>江苏测试科技有限公司</BuyerName></BuyerInformation>
          <BasicInformation><TotalAmWithoutTax>100.00</TotalAmWithoutTax><TotalTaxAm>13.00</TotalTaxAm><TotalTax-includedAmount>113.00</TotalTax-includedAmount></BasicInformation>
          <TaxSupervisionInfo><InvoiceNumber>25322000000337005189</InvoiceNumber><IssueTime>2025-07-22T08:00:00</IssueTime></TaxSupervisionInfo>
        </EInvoice>"#).unwrap();

        let result = extract_file(&path).unwrap();
        let _ = fs::remove_file(path);
        assert!(result.success);
        assert_eq!(result.page_count, 1);
        assert_eq!(result.invoices[0].seller_name, "无锡示例商贸有限公司");
        assert_eq!(result.invoices[0].amount_tax, 113.0);
    }

    #[test]
    fn replaces_unknown_type_when_later_source_identifies_invoice() {
        let mut incomplete = InvoiceInfo {
            invoice_type: "unknown".to_string(),
            ..Default::default()
        };
        incomplete.merge_missing(InvoiceInfo {
            invoice_type: "vat-general".to_string(),
            ..Default::default()
        });

        assert_eq!(incomplete.invoice_type, "vat-general");
    }

    #[test]
    fn merge_keeps_pdf_and_ocr_raw_text() {
        let mut incomplete = InvoiceInfo {
            raw_text: Some("PDF template".to_string()),
            ..Default::default()
        };
        incomplete.merge_missing(InvoiceInfo {
            raw_text: Some("OCR values".to_string()),
            ..Default::default()
        });

        assert_eq!(
            incomplete.raw_text.as_deref(),
            Some("PDF template\nOCR values")
        );
    }

    #[test]
    fn file_name_fallback_prefers_longest_invoice_number() {
        let path = Path::new("餐饮_20260817_26437000000232262330.pdf");
        assert_eq!(
            invoice_number_from_file_name(path).as_deref(),
            Some("26437000000232262330")
        );
    }

    #[test]
    fn recursively_extracts_supported_files_and_keeps_failures() {
        let root = std::env::temp_dir().join(format!(
            "invoice-extractor-directory-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            nested.join("invoice.xml"),
            "<EInvoice><InvoiceNumber>25322000000337005189</InvoiceNumber></EInvoice>",
        )
        .unwrap();
        fs::write(root.join("broken.pdf"), b"not a pdf").unwrap();
        fs::write(root.join("ignored.txt"), b"not an invoice").unwrap();

        let result = extract_directory(&root).unwrap();
        let _ = fs::remove_dir_all(root);

        assert!(!result.success);
        assert_eq!(result.matched_file_count, 2);
        assert_eq!(result.extracted_file_count, 1);
        assert_eq!(result.failed_file_count, 1);
        assert_eq!(result.files[0].file_name, "invoice.xml");
        assert!(result.errors[0].file_path.ends_with("broken.pdf"));
    }
}
