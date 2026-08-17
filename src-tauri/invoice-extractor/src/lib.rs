mod backend;
mod extractor;
mod formats;
mod model;
#[cfg(feature = "paddle-ocr")]
mod native_pdf;
#[cfg(feature = "paddle-ocr")]
mod paddle;
mod parser;

pub use backend::{NoOcr, OcrBackend};
pub use extractor::{extract_directory, extract_file, InvoiceExtractor};
pub use model::{
    AmountValidation, DirectoryExtractionError, ExtractionOptions, InvoiceDirectoryResult,
    InvoiceFileResult, InvoiceInfo, RecognitionLine, RecognitionPage, RecognitionWord,
};
#[cfg(feature = "paddle-ocr")]
pub use native_pdf::NativePdfRenderer;
#[cfg(feature = "paddle-ocr")]
pub use paddle::{NoPdfRenderer, PaddleOcrBackend, PdfPageRenderer};
