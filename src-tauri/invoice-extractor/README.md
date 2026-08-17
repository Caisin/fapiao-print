# invoice-extractor

跨平台发票文件信息提取库。核心解析不依赖 Tauri；可选的 macOS 原生 PDF
renderer 单独隔离在 `native_pdf.rs`，其他平台仍可通过公共 trait 注入实现。

## 模块

- `model.rs`: 公共输入、输出和识别坐标模型
- `formats/`: PDF、OFD、XML 文件读取
- `parser/`: 字段、金额、文本标准化与校验
- `extractor.rs`: 文件类型分发和多页结果合并
- `backend.rs`: OCR 后端接口
- `paddle.rs`: 可选 PP-OCRv5/MNN 后端和 PDF 页面渲染接口
- `native_pdf.rs`: 可选 macOS Core Graphics PDF 页面渲染器

## 使用

### PDF 文字层、OFD、XML

PDF 解析会递归展开页面调用的 Form XObject，并使用每个 Form 自带的字体资源
读取 WPS/Adobe 中可编辑的实际字段文字。文字层已包含核心字段时不会启动 OCR。

```rust
let result = invoice_extractor::extract_file("invoice.xml")?;
println!("发票数量: {}", result.invoices.len());

for invoice in &result.invoices {
    println!("票号: {}", invoice.invoice_no);
    println!("发票类型: {}", invoice.invoice_type);
    println!("销售方: {}", invoice.seller_name);
    println!("含税金额: {}", invoice.amount_tax);
    println!("税率: {}", invoice.tax_rate);
    println!("大写金额: {}", invoice.amount_uppercase);
    println!("开票人: {}", invoice.invoice_clerk);
}
```

`invoice_type`（JSON 为 `invoiceType`）使用稳定枚举值：增值税普通发票为
`vat-general`，增值税专用发票为 `vat-special`，非税票据为 `nontax`；仅在
票面没有明确普票/专票字样时，铁路、航空和其他运输票据才分别返回 `train`、
`flight`、`ticket`。运输用途同时由 `is_ticket`（JSON 为 `isTicket`）独立标记，
因此普通电子运输发票会返回 `invoiceType: "vat-general"` 和 `isTicket: true`。

`tax_rate` 保留百分号或税务语义（如 `13%`、`免税`、`不征税`）；多税率
发票按票面出现顺序使用英文逗号连接。

`amount_uppercase` 保留票面中文大写金额，`invoice_clerk` 返回开票人姓名。

`amount_validation` 仅在金额关系异常且无法自动修复时返回原始金额快照；正常满足
`amount_tax = amount_no_tax + tax_amount`、无需校验的票种或已自动修复时均为
`null`。因此 `null` 表示“没有未解决的金额异常”，不是“未执行解析”。

带坐标文字层的 PDF 还会返回 `line_items`（JSON 为 `lineItems`）商品明细：

```json
{
  "projectName": "*其他食品*素牛筋20g",
  "specification": "11011771",
  "unit": "个",
  "quantity": 2.0,
  "unitPrice": 0.885,
  "amount": 1.77,
  "taxRate": "13%",
  "taxAmount": 0.23,
  "amountTax": 2.0,
  "isDiscount": false
}
```

折扣行同样保留为独立明细，`isDiscount` 为 `true`；票面未提供的规格、单位、
数量和单价分别返回空字符串或 `null`。多页同一发票会按发票号码同步最终汇总
金额，明细仍归属其实际出现的页面。

可直接运行仓库示例：

```bash
cargo run \
  --manifest-path src-tauri/invoice-extractor/Cargo.toml \
  --example extract_file -- \
  /absolute/path/to/invoice.xml
```

### 递归解析目录

无 OCR 场景可递归解析目录内的 PDF 文字层、OFD 和 XML：

```rust
let batch = invoice_extractor::extract_directory("/absolute/path/to/invoices")?;
println!("匹配: {}, 成功: {}, 失败: {}",
    batch.matched_file_count,
    batch.extracted_file_count,
    batch.failed_file_count,
);
for file in &batch.files {
    println!("{}: {} 张发票", file.file_name, file.invoices.len());
}
for failure in &batch.errors {
    eprintln!("{}: {}", failure.file_path, failure.error);
}
```

```bash
cargo run \
  --manifest-path src-tauri/invoice-extractor/Cargo.toml \
  --example extract_directory -- \
  /absolute/path/to/invoices
```

目录扫描支持 PDF、OFD、XML、JPG/JPEG、PNG、BMP、WebP、TIFF，忽略其他
文件且不跟随目录符号链接。结果按文件路径排序；单个文件失败只写入 `errors[]`。

### 图片 OCR

启用 `paddle-ocr` feature 后，可直接使用仓库现有 PP-OCRv5 模型：

```rust
use invoice_extractor::{
    ExtractionOptions, InvoiceExtractor, NativePdfRenderer, PaddleOcrBackend,
};

let backend = PaddleOcrBackend::with_renderer(
    "src-tauri/models",
    NativePdfRenderer,
)?;
let result = invoice_extractor::InvoiceExtractor::new(backend)
    .extract_file("invoice.jpg", ExtractionOptions::default())?;
```

运行完整示例。第一个参数也可以是目录，此时会递归解析并复用同一 OCR 引擎：

```bash
cargo run \
  --manifest-path src-tauri/invoice-extractor/Cargo.toml \
  --features paddle-ocr \
  --example extract_with_ocr -- \
  /absolute/path/to/invoice.jpg \
  src-tauri/models \
  precise
```

模型目录和识别精度均可省略；精度支持 `fast`、`standard`、`precise`。

模型目录必须包含：

- `PP-OCRv5_mobile_det.mnn`
- `PP-OCRv5_mobile_rec.mnn`
- `ppocr_keys_v5.txt`

`NativePdfRenderer` 在 macOS 上通过系统 Core Graphics 渲染 PDF，无需安装
PDFium 或其他运行库；普通图片仍由 `PaddleOcrBackend` 直接读取。其他平台若
需要独立解析扫描 PDF，可按下节注入宿主渲染器。

### 扫描 PDF

macOS 可直接使用上面的 `NativePdfRenderer`。其他平台的扫描 PDF 需要宿主提供
页面渲染器，OCR 和字段解析仍由 crate 完成：

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

递归解析目录：

```js
var batch = await extractInvoiceDirectory('/absolute/path/to/invoices', {
  useOcr: true,
  ocrPrecision: 'standard',
  includeRawText: false
});

console.log(batch.files, batch.errors);
```
