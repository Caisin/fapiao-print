use invoice_extractor::{ExtractionOptions, InvoiceExtractor, NativePdfRenderer, PaddleOcrBackend};

fn main() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let file_path = arguments.next().ok_or_else(|| {
        "用法: extract_with_ocr <发票文件或目录> [模型目录] [识别精度]".to_string()
    })?;
    let model_dir = arguments
        .next()
        .unwrap_or_else(|| "src-tauri/models".to_string());
    let precision = arguments.next().unwrap_or_else(|| "standard".to_string());

    // NativePdfRenderer uses Core Graphics on macOS. Other platforms can
    // inject their renderer through PaddleOcrBackend::with_renderer.
    let backend = PaddleOcrBackend::with_renderer(model_dir, NativePdfRenderer)?;
    let extractor = InvoiceExtractor::new(backend);
    let options = ExtractionOptions {
        use_ocr: true,
        ocr_precision: precision,
        include_raw_text: true,
    };

    let path = std::path::Path::new(&file_path);
    let result = if path.is_dir() {
        serde_json::to_value(extractor.extract_directory(path, options)?)
    } else {
        serde_json::to_value(extractor.extract_file(path, options)?)
    }
    .map_err(|error| error.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?
    );
    Ok(())
}
