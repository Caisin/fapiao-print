use invoice_extractor::extract_directory;

fn main() -> Result<(), String> {
    let directory_path = std::env::args()
        .nth(1)
        .ok_or_else(|| "用法: extract_directory <发票目录>".to_string())?;

    let result = extract_directory(directory_path)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?
    );
    Ok(())
}
