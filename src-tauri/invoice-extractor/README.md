# invoice-extractor

平台无关的发票文件信息提取库。crate 不依赖 Tauri，也不包含 Windows 或 macOS API。

## 模块

- `model.rs`: 公共输入、输出和识别坐标模型
- `formats/`: PDF、OFD、XML 文件读取
- `parser/`: 字段、金额、文本标准化与校验
- `extractor.rs`: 文件类型分发和多页结果合并
- `backend.rs`: OCR 后端接口
- `paddle.rs`: 可选 PP-OCRv5/MNN 后端和 PDF 页面渲染接口

## 使用

### PDF 文字层、OFD、XML

```rust
let result = invoice_extractor::extract_file("invoice.xml")?;
println!("发票数量: {}", result.invoices.len());

for invoice in &result.invoices {
    println!("票号: {}", invoice.invoice_no);
    println!("销售方: {}", invoice.seller_name);
    println!("含税金额: {}", invoice.amount_tax);
}
```

可直接运行仓库示例：

```bash
cargo run \
  --manifest-path src-tauri/invoice-extractor/Cargo.toml \
  --example extract_file -- \
  /absolute/path/to/invoice.xml
```

### 图片 OCR

启用 `paddle-ocr` feature 后，可直接使用仓库现有 PP-OCRv5 模型：

```rust
use invoice_extractor::{ExtractionOptions, InvoiceExtractor, PaddleOcrBackend};

let backend = PaddleOcrBackend::from_model_dir("src-tauri/models")?;
let result = invoice_extractor::InvoiceExtractor::new(backend)
    .extract_file("invoice.jpg", ExtractionOptions::default())?;
```

运行完整示例：

```bash
cargo run \
  --manifest-path src-tauri/invoice-extractor/Cargo.toml \
  --features paddle-ocr \
  --example extract_with_ocr -- \
  /absolute/path/to/invoice.jpg \
  src-tauri/models
```

模型目录必须包含：

- `PP-OCRv5_mobile_det.mnn`
- `PP-OCRv5_mobile_rec.mnn`
- `ppocr_keys_v5.txt`

宿主仅需为扫描 PDF 实现 `PdfPageRenderer`。普通图片由 `PaddleOcrBackend`
直接读取，字段识别和结果结构不依赖操作系统。

### 扫描 PDF

扫描 PDF 需要宿主提供页面渲染器，OCR 和字段解析仍由 crate 完成：

```rust
use image::DynamicImage;
use invoice_extractor::{
    ExtractionOptions, InvoiceExtractor, PaddleOcrBackend, PdfPageRenderer,
};
use std::path::Path;

struct MyPdfRenderer;

impl PdfPageRenderer for MyPdfRenderer {
    fn render_page(
        &self,
        pdf_path: &Path,
        page_index: u32,
        dpi: u32,
    ) -> Result<DynamicImage, String> {
        // 在这里调用宿主的 CoreGraphics、WinRT、PDFium 或其他 PDF 渲染器。
        todo!("render {pdf_path:?} page {page_index} at {dpi} DPI")
    }
}

let backend = PaddleOcrBackend::with_renderer("src-tauri/models", MyPdfRenderer)?;
let extractor = InvoiceExtractor::new(backend);
let result = extractor.extract_file("scan.pdf", ExtractionOptions::default())?;
```

Tauri 应用中的实现位于 `src-tauri/src/lib.rs`，Windows 与 macOS 共用
`PdfPageRenderer` 接口，并缓存一次渲染得到的所有页面。

### 前端调用

Tauri WebView 中可以直接调用全局方法：

```js
var result = await extractInvoiceFile('/absolute/path/to/invoice.pdf', {
  useOcr: true,
  ocrPrecision: 'standard',
  includeRawText: true
});

console.log(result.invoices);
```
