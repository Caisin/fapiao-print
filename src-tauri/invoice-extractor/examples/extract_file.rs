use invoice_extractor::extract_file;

fn main() -> Result<(), String> {
    let file_path = std::env::args()
        .nth(1)
        .ok_or_else(|| "用法: extract_file <PDF/OFD/XML 文件路径>".to_string())?;

    let result = extract_file(file_path)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?
    );
    Ok(())
}
