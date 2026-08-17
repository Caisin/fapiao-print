use crate::RecognitionPage;
use std::path::Path;

pub trait OcrBackend: Send + Sync {
    fn is_available(&self) -> bool;

    fn recognize_image(&self, path: &Path, precision: &str) -> Result<RecognitionPage, String>;

    fn recognize_pdf_page(
        &self,
        path: &Path,
        page_index: u32,
        precision: &str,
    ) -> Result<RecognitionPage, String>;
}

impl<T: OcrBackend + ?Sized> OcrBackend for &T {
    fn is_available(&self) -> bool {
        (**self).is_available()
    }

    fn recognize_image(&self, path: &Path, precision: &str) -> Result<RecognitionPage, String> {
        (**self).recognize_image(path, precision)
    }

    fn recognize_pdf_page(
        &self,
        path: &Path,
        page_index: u32,
        precision: &str,
    ) -> Result<RecognitionPage, String> {
        (**self).recognize_pdf_page(path, page_index, precision)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoOcr;

impl OcrBackend for NoOcr {
    fn is_available(&self) -> bool {
        false
    }

    fn recognize_image(&self, _path: &Path, _precision: &str) -> Result<RecognitionPage, String> {
        Err("当前提取器未配置 OCR 后端".to_string())
    }

    fn recognize_pdf_page(
        &self,
        _path: &Path,
        _page_index: u32,
        _precision: &str,
    ) -> Result<RecognitionPage, String> {
        Err("当前提取器未配置 OCR 后端".to_string())
    }
}
