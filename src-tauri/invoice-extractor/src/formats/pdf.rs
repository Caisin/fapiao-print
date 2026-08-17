use crate::RecognitionPage;
use lopdf::Document;
use std::path::Path;

pub(crate) struct PdfReadResult {
    pub pages: Vec<RecognitionPage>,
    pub warnings: Vec<String>,
}

pub(crate) fn read_pdf_pages(path: &Path) -> Result<PdfReadResult, String> {
    let mut document = Document::load(path).map_err(|error| format!("PDF 加载失败: {error}"))?;
    let repaired_cmaps = repair_nonstandard_tounicode_cmaps(&mut document);
    let pages = document.get_pages();
    let mut result = Vec::with_capacity(pages.len());
    let mut warnings = Vec::new();
    if repaired_cmaps > 0 {
        warnings.push(format!(
            "已兼容修复 {repaired_cmaps} 个非标准 ToUnicode CMap"
        ));
    }

    for page_number in pages.keys() {
        let (text, warning) = extract_page_text(&document, *page_number);
        warnings.extend(warning);
        result.push(RecognitionPage::from_text(text));
    }
    Ok(PdfReadResult {
        pages: result,
        warnings,
    })
}

fn extract_page_text(document: &Document, page_number: u32) -> (String, Option<String>) {
    match document.extract_text(&[page_number]) {
        Ok(text) => (text, None),
        Err(error) => {
            let partial_text = document
                .extract_text_chunks(&[page_number])
                .into_iter()
                .filter_map(Result::ok)
                .collect::<String>();
            let warning = if partial_text.trim().is_empty() {
                format!("PDF 第 {page_number} 页文字无法解码，将尝试 OCR: {error}")
            } else {
                format!("PDF 第 {page_number} 页部分文字无法解码，已保留可识别内容: {error}")
            };
            (partial_text, Some(warning))
        }
    }
}

fn repair_nonstandard_tounicode_cmaps(document: &mut Document) -> usize {
    let mut repaired_count = 0;
    for object in document.objects.values_mut() {
        let Some(stream) = object.as_stream_mut().ok() else {
            continue;
        };
        let Ok(content) = stream.get_plain_content() else {
            continue;
        };
        if !content
            .windows(b"begincmap".len())
            .any(|window| window == b"begincmap")
        {
            continue;
        }
        let repaired = replace_bytes(&content, b"difineresource", b"defineresource");
        if repaired != content {
            stream.set_plain_content(repaired);
            repaired_count += 1;
        }
    }
    repaired_count
}

fn replace_bytes(input: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut offset = 0;
    while let Some(position) = input[offset..]
        .windows(from.len())
        .position(|window| window == from)
    {
        let position = offset + position;
        output.extend_from_slice(&input[offset..position]);
        output.extend_from_slice(to);
        offset = position + from.len();
    }
    output.extend_from_slice(&input[offset..]);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Object, Stream, StringFormat};

    #[test]
    fn repairs_misspelled_defineresource_in_tounicode_cmap() {
        let mut document = document_with_cmap(
            b"1 beginbfchar\n<0001> <53D1>\nendbfchar\n\
              endcmap\nCMapName currentdict /CMap difineresource pop\nend\nend",
        );

        assert!(document.extract_text(&[1]).is_err());
        assert_eq!(repair_nonstandard_tounicode_cmaps(&mut document), 1);
        assert_eq!(document.extract_text(&[1]).unwrap().trim(), "发");
    }

    #[test]
    fn keeps_page_available_when_cmap_cannot_be_repaired() {
        let document = document_with_cmap(b"this is not a valid CMap");

        let (text, warning) = extract_page_text(&document, 1);

        assert!(text.trim().is_empty());
        assert!(warning.unwrap().contains("将尝试 OCR"));
    }

    fn document_with_cmap(mapping: &[u8]) -> Document {
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let mut cmap = b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
            /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def\n\
            /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
            1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n"
            .to_vec();
        cmap.extend_from_slice(mapping);
        let cmap_id = document.add_object(Stream::new(dictionary! {}, cmap));
        let font_id = document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type0",
            "BaseFont" => "TestFont",
            "Encoding" => "Identity-H",
            "ToUnicode" => Object::Reference(cmap_id),
        });
        let resources_id = document.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new(
                    "Tj",
                    vec![Object::String(vec![0, 1], StringFormat::Hexadecimal)],
                ),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = document.add_object(Stream::new(
            dictionary! {},
            content.encode().expect("encode content"),
        ));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Resources" => resources_id,
            "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        document
    }
}
