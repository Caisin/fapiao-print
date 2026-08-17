use crate::{OcrBackend, RecognitionLine, RecognitionPage, RecognitionWord};
use image::DynamicImage;
use ocr_rs::{DetOptions, OcrEngine, OcrEngineConfig, RecOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

const DET_MODEL: &str = "PP-OCRv5_mobile_det.mnn";
const REC_MODEL: &str = "PP-OCRv5_mobile_rec.mnn";
const CHARSET: &str = "ppocr_keys_v5.txt";

pub trait PdfPageRenderer: Send + Sync {
    fn render_page(
        &self,
        pdf_path: &Path,
        page_index: u32,
        dpi: u32,
    ) -> Result<DynamicImage, String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoPdfRenderer;

impl PdfPageRenderer for NoPdfRenderer {
    fn render_page(
        &self,
        _pdf_path: &Path,
        _page_index: u32,
        _dpi: u32,
    ) -> Result<DynamicImage, String> {
        Err("当前 OCR 后端未配置 PDF 页面渲染器".to_string())
    }
}

pub struct PaddleOcrBackend<R = NoPdfRenderer> {
    model_dir: PathBuf,
    renderer: R,
    engine: Mutex<Option<OcrEngine>>,
}

impl PaddleOcrBackend<NoPdfRenderer> {
    pub fn from_model_dir(model_dir: impl Into<PathBuf>) -> Result<Self, String> {
        Self::with_renderer(model_dir, NoPdfRenderer)
    }
}

impl<R: PdfPageRenderer> PaddleOcrBackend<R> {
    pub fn with_renderer(model_dir: impl Into<PathBuf>, renderer: R) -> Result<Self, String> {
        let model_dir = model_dir.into();
        validate_models(&model_dir)?;
        Ok(Self {
            model_dir,
            renderer,
            engine: Mutex::new(None),
        })
    }

    pub fn model_dir(&self) -> &Path {
        &self.model_dir
    }

    pub fn recognize_dynamic_image(
        &self,
        image: DynamicImage,
        precision: &str,
    ) -> Result<RecognitionPage, String> {
        let max_dimension = match precision {
            "fast" => 1280,
            "precise" => 2800,
            _ => 1920,
        };
        self.recognize_dynamic_image_with_limit(image, precision, max_dimension)
    }

    fn recognize_dynamic_image_with_limit(
        &self,
        image: DynamicImage,
        precision: &str,
        max_dimension: u32,
    ) -> Result<RecognitionPage, String> {
        let original_width = image.width();
        let original_height = image.height();
        if original_width == 0 || original_height == 0 {
            return Err("OCR 图片尺寸为空".to_string());
        }
        let image = resize_for_ocr(image, max_dimension);
        let resized_width = image.width();
        let resized_height = image.height();
        let scale_x = original_width as f64 / resized_width as f64;
        let scale_y = original_height as f64 / resized_height as f64;

        let engine = self.engine()?;
        let ocr_engine = engine
            .as_ref()
            .ok_or_else(|| "OCR 引擎未初始化".to_string())?;
        let results = ocr_engine
            .recognize(&image)
            .map_err(|error| format!("PaddleOCR 识别失败: {error:?}"))?;

        let mut lines = Vec::with_capacity(results.len());
        let mut text = Vec::with_capacity(results.len());
        for result in results {
            let mut line_text = result.text.trim().to_string();
            if line_text.is_empty() {
                continue;
            }
            let rect = result.bbox.rect;
            if let Some(refined) = refine_structured_line(
                ocr_engine,
                &image,
                &line_text,
                rect.left(),
                rect.top(),
                rect.right(),
                rect.bottom(),
            ) {
                line_text = refined;
            }
            if precision != "fast" && should_refine_name_line(&line_text) {
                if let Some(refined) = refine_name_line(
                    ocr_engine,
                    &image,
                    rect.left(),
                    rect.top(),
                    rect.right(),
                    rect.bottom(),
                ) {
                    if refined.chars().count() > line_text.chars().count() {
                        line_text = refined;
                    }
                }
            }
            text.push(line_text.clone());
            lines.push(RecognitionLine {
                words: vec![RecognitionWord {
                    text: line_text,
                    x: rect.left() as f64 * scale_x,
                    y: rect.top() as f64 * scale_y,
                    w: (rect.right() - rect.left()) as f64 * scale_x,
                    h: (rect.bottom() - rect.top()) as f64 * scale_y,
                }],
                confidence: result.confidence,
            });
        }
        drop(engine);

        Ok(RecognitionPage {
            text: text.join("\n"),
            lines,
            img_w: original_width,
            img_h: original_height,
        })
    }

    fn engine(&self) -> Result<MutexGuard<'_, Option<OcrEngine>>, String> {
        let mut engine = self
            .engine
            .lock()
            .map_err(|error| format!("OCR 引擎锁失败: {error}"))?;
        if engine.is_none() {
            let config = OcrEngineConfig::new()
                .with_parallel(false)
                .with_threads(4)
                .with_min_result_confidence(0.3)
                .with_det_options(DetOptions::new().with_max_side_len(1920))
                .with_rec_options(RecOptions::new().with_batch_size(16).with_batch(true));
            *engine = Some(
                OcrEngine::new(
                    self.model_dir.join(DET_MODEL),
                    self.model_dir.join(REC_MODEL),
                    self.model_dir.join(CHARSET),
                    Some(config),
                )
                .map_err(|error| format!("创建 PaddleOCR 引擎失败: {error:?}"))?,
            );
        }
        Ok(engine)
    }
}

fn refine_structured_line(
    engine: &OcrEngine,
    image: &DynamicImage,
    text: &str,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
) -> Option<String> {
    #[derive(Clone, Copy)]
    enum Field {
        InvoiceNumber,
        CreditCode,
        Date,
    }

    let field = if text.contains("发票号码") || text.contains("发票号") {
        Field::InvoiceNumber
    } else if text.contains("信用代码") || text.contains("识别号") {
        Field::CreditCode
    } else if text.contains("开票日期") {
        Field::Date
    } else {
        return None;
    };
    let existing = match field {
        Field::InvoiceNumber => longest_ascii_run(text, true),
        Field::CreditCode => longest_ascii_run(text, false),
        Field::Date => text
            .chars()
            .filter(char::is_ascii_digit)
            .collect::<String>(),
    };
    let existing_is_complete = match field {
        Field::InvoiceNumber => (8..=20).contains(&existing.len()),
        Field::CreditCode => (15..=20).contains(&existing.len()),
        Field::Date => existing.len() == 8,
    };
    if existing_is_complete {
        return None;
    }
    let width = (right - left).max(1) as u32;
    let height = (bottom - top).max(1) as u32;
    let value_fraction = match field {
        Field::InvoiceNumber => 0.62,
        Field::CreditCode => 0.46,
        Field::Date => 0.58,
    };
    let value_width = (width as f64 * value_fraction) as u32;
    let pad_x = (width / 50).max(4);
    let pad_y = (height / 3).max(4);
    let x = (right.max(0) as u32)
        .saturating_sub(value_width)
        .saturating_sub(pad_x);
    let y = (top.max(0) as u32).saturating_sub(pad_y);
    let crop_width = value_width
        .saturating_add(pad_x * 2)
        .min(image.width().saturating_sub(x));
    let crop_height = height
        .saturating_add(pad_y * 2)
        .min(image.height().saturating_sub(y));
    if crop_width == 0 || crop_height == 0 {
        return None;
    }
    let crop = image.crop_imm(x, y, crop_width, crop_height);
    let recognized = engine.recognize_text(&crop).ok()?.text;
    match field {
        Field::InvoiceNumber => {
            let value = longest_ascii_run(&recognized, true);
            (8..=20)
                .contains(&value.len())
                .then(|| format!("发票号码：{value}"))
        }
        Field::CreditCode => {
            let value = longest_ascii_run(&recognized, false);
            (15..=20)
                .contains(&value.len())
                .then(|| format!("统一社会信用代码/纳税人识别号：{value}"))
        }
        Field::Date => {
            let digits = recognized
                .chars()
                .filter(char::is_ascii_digit)
                .collect::<String>();
            (digits.len() == 8).then(|| {
                format!(
                    "开票日期：{}年{}月{}日",
                    &digits[..4],
                    &digits[4..6],
                    &digits[6..]
                )
            })
        }
    }
}

fn longest_ascii_run(text: &str, digits_only: bool) -> String {
    text.split(|character: char| {
        if digits_only {
            !character.is_ascii_digit()
        } else {
            !character.is_ascii_alphanumeric()
        }
    })
    .max_by_key(|value| value.len())
    .unwrap_or_default()
    .to_ascii_uppercase()
}

fn should_refine_name_line(text: &str) -> bool {
    text.contains("名称")
        && ["公司", "企业", "中心", "商行", "商店", "经营部"]
            .iter()
            .any(|suffix| text.contains(suffix))
}

fn refine_name_line(
    engine: &OcrEngine,
    image: &DynamicImage,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
) -> Option<String> {
    let width = (right - left).max(1) as u32;
    let height = (bottom - top).max(1) as u32;
    let pad_x = (width / 20).max(8);
    let pad_y = (height / 2).max(8);
    let x = left.max(0) as u32;
    let y = top.max(0) as u32;
    let x = x.saturating_sub(pad_x);
    let y = y.saturating_sub(pad_y);
    let crop_width = width
        .saturating_add(pad_x * 2)
        .min(image.width().saturating_sub(x));
    let crop_height = height
        .saturating_add(pad_y * 2)
        .min(image.height().saturating_sub(y));
    if crop_width == 0 || crop_height == 0 {
        return None;
    }
    let crop = image.crop_imm(x, y, crop_width, crop_height).resize_exact(
        crop_width.saturating_mul(2),
        crop_height.saturating_mul(2),
        image::imageops::FilterType::Lanczos3,
    );
    let candidate = engine
        .recognize(&crop)
        .ok()?
        .into_iter()
        .map(|result| result.text.trim().to_string())
        .filter(|text| !text.is_empty())
        .collect::<String>();
    should_refine_name_line(&candidate).then_some(candidate)
}

impl<R: PdfPageRenderer> OcrBackend for PaddleOcrBackend<R> {
    fn is_available(&self) -> bool {
        true
    }

    fn recognize_image(&self, path: &Path, precision: &str) -> Result<RecognitionPage, String> {
        let image = image::open(path).map_err(|error| format!("读取 OCR 图片失败: {error}"))?;
        self.recognize_dynamic_image(image, precision)
    }

    fn recognize_pdf_page(
        &self,
        path: &Path,
        page_index: u32,
        precision: &str,
    ) -> Result<RecognitionPage, String> {
        let max_dimension = match precision {
            "fast" => 1280,
            "precise" => 2800,
            _ => 1920,
        };
        let image = self.renderer.render_page(path, page_index, 192)?;
        self.recognize_dynamic_image_with_limit(image, precision, max_dimension)
    }
}

fn validate_models(model_dir: &Path) -> Result<(), String> {
    for file_name in [DET_MODEL, REC_MODEL, CHARSET] {
        let path = model_dir.join(file_name);
        if !path.is_file() {
            return Err(format!("OCR 模型文件不存在: {}", path.display()));
        }
    }
    Ok(())
}

fn resize_for_ocr(image: DynamicImage, max_dimension: u32) -> DynamicImage {
    let longest = image.width().max(image.height());
    if longest <= max_dimension {
        return image;
    }
    let scale = max_dimension as f64 / longest as f64;
    let width = (image.width() as f64 * scale).round().max(1.0) as u32;
    let height = (image.height() as f64 * scale).round().max(1.0) as u32;
    image.resize_exact(width, height, image::imageops::FilterType::Lanczos3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_repo_models_and_runs_inference() {
        let model_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../models");
        if !model_dir.is_dir() {
            return;
        }
        let backend = PaddleOcrBackend::from_model_dir(model_dir).unwrap();
        let image = DynamicImage::new_rgb8(64, 64);
        let result = backend.recognize_dynamic_image(image, "fast").unwrap();
        assert_eq!(result.img_w, 64);
        assert_eq!(result.img_h, 64);
    }
}
