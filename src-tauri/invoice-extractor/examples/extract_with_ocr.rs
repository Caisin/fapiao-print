use invoice_extractor::{ExtractionOptions, InvoiceExtractor, PaddleOcrBackend};

fn main() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let file_path = arguments.next().ok_or_else(|| {
        "用法: extract_with_ocr <发票文件或目录> [模型目录] [识别精度]".to_string()
    })?;
    let model_dir = arguments
        .next()
        .unwrap_or_else(|| "src-tauri/models".to_string());
    let precision = arguments.next().unwrap_or_else(|| "standard".to_string());

    // No PDF renderer is needed for JPG/PNG/BMP/WebP/TIFF invoice images.
    let backend = PaddleOcrBackend::from_model_dir(model_dir)?;
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
