use crate::{RecognitionLine, RecognitionPage, RecognitionWord};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use std::collections::{HashMap, HashSet};
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

    for (page_number, page_id) in pages {
        let (mut text, warning) = extract_page_text(&document, page_number);
        warnings.extend(warning);
        let (form_texts, form_warnings) = extract_form_texts(&document, page_id, page_number);
        warnings.extend(form_warnings);
        let mut lines = Vec::new();
        for form_text in form_texts {
            if !form_text.text.trim().is_empty() && !text.contains(form_text.text.trim()) {
                if !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&form_text.text);
            }
            for value in form_text
                .text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
            {
                lines.push(RecognitionLine {
                    words: vec![RecognitionWord {
                        text: value.to_string(),
                        x: form_text.x,
                        y: form_text.y,
                        w: 0.0,
                        h: 0.0,
                    }],
                    confidence: 1.0,
                });
            }
        }
        let (img_w, img_h) = page_dimensions(&document, page_id).unwrap_or((0, 0));
        result.push(RecognitionPage {
            text,
            lines,
            img_w,
            img_h,
        });
    }
    Ok(PdfReadResult {
        pages: result,
        warnings,
    })
}

struct FormPage {
    content: Vec<u8>,
    resources: Object,
    x: f64,
    y: f64,
}

struct FormText {
    text: String,
    x: f64,
    y: f64,
}

fn extract_form_texts(
    document: &Document,
    page_id: ObjectId,
    page_number: u32,
) -> (Vec<FormText>, Vec<String>) {
    let Some(page_resources) = inherited_page_resources(document, page_id) else {
        return (Vec::new(), Vec::new());
    };
    let Ok(page_content) = document.get_and_decode_page_content(page_id) else {
        return (Vec::new(), Vec::new());
    };
    let mut forms = Vec::new();
    collect_form_pages(
        document,
        &page_content.operations,
        &page_resources,
        [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        &mut HashSet::new(),
        &mut forms,
    );

    let mut texts = Vec::with_capacity(forms.len());
    let mut warnings = Vec::new();
    for (index, form) in forms.into_iter().enumerate() {
        let position = (form.x, form.y);
        match extract_form_text(document, page_id, page_number, form) {
            Ok(text) if !text.trim().is_empty() => texts.push(FormText {
                text,
                x: position.0,
                y: position.1,
            }),
            Ok(_) => {}
            Err(error) => warnings.push(format!(
                "PDF 第 {page_number} 页 Form XObject {} 文字提取失败: {error}",
                index + 1
            )),
        }
    }
    (texts, warnings)
}

fn inherited_page_resources(document: &Document, page_id: ObjectId) -> Option<Object> {
    let mut current = page_id;
    let mut visited = HashSet::new();
    while visited.insert(current) {
        let dictionary = document.get_dictionary(current).ok()?;
        if let Ok(resources) = dictionary.get(b"Resources") {
            return Some(resources.clone());
        }
        current = dictionary.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn collect_form_pages(
    document: &Document,
    operations: &[lopdf::content::Operation],
    resources: &Object,
    initial_matrix: [f64; 6],
    visited: &mut HashSet<ObjectId>,
    forms: &mut Vec<FormPage>,
) {
    let Some(resources_dictionary) = resolve_dictionary(document, resources) else {
        return;
    };
    let Ok(xobjects) = resources_dictionary.get(b"XObject") else {
        return;
    };
    let Some(xobject_dictionary) = resolve_dictionary(document, xobjects) else {
        return;
    };

    let names = xobject_dictionary
        .iter()
        .map(|(name, object)| (name.as_slice(), object))
        .collect::<HashMap<_, _>>();
    let identity = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut current = initial_matrix;
    let mut stack = Vec::new();
    for operation in operations {
        match operation.operator.as_str() {
            "q" => stack.push(current),
            "Q" => current = stack.pop().unwrap_or(initial_matrix),
            "cm" if operation.operands.len() == 6 => {
                let values = operation
                    .operands
                    .iter()
                    .map(object_number)
                    .collect::<Option<Vec<_>>>();
                if let Some(values) = values {
                    current = multiply_matrix(
                        current,
                        [
                            values[0], values[1], values[2], values[3], values[4], values[5],
                        ],
                    );
                }
            }
            "Do" => {
                let object = operation
                    .operands
                    .first()
                    .and_then(|object| object.as_name().ok())
                    .and_then(|name| names.get(name))
                    .copied();
                let Some(object) = object else { continue };
                let Some((object_id, stream)) = resolve_stream(document, object) else {
                    continue;
                };
                if !visited.insert(object_id)
                    || stream.dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Form")
                {
                    continue;
                }
                let content = stream
                    .decompressed_content()
                    .unwrap_or_else(|_| stream.content.clone());
                let form_resources = stream
                    .dict
                    .get(b"Resources")
                    .cloned()
                    .unwrap_or_else(|_| resources.clone());
                let form_matrix = stream
                    .dict
                    .get(b"Matrix")
                    .ok()
                    .and_then(object_matrix)
                    .unwrap_or(identity);
                let form_position = multiply_matrix(current, form_matrix);
                forms.push(FormPage {
                    content: content.clone(),
                    resources: form_resources.clone(),
                    x: form_position[4],
                    y: form_position[5],
                });
                if let Ok(form_content) = lopdf::content::Content::decode(&content) {
                    collect_form_pages(
                        document,
                        &form_content.operations,
                        &form_resources,
                        form_position,
                        visited,
                        forms,
                    );
                }
            }
            _ => {}
        }
    }
}

fn object_matrix(object: &Object) -> Option<[f64; 6]> {
    let values = object
        .as_array()
        .ok()?
        .iter()
        .map(object_number)
        .collect::<Option<Vec<_>>>()?;
    (values.len() == 6).then(|| {
        [
            values[0], values[1], values[2], values[3], values[4], values[5],
        ]
    })
}

fn object_number(object: &Object) -> Option<f64> {
    match object {
        Object::Integer(value) => Some(*value as f64),
        Object::Real(value) => Some(*value as f64),
        _ => None,
    }
}

fn multiply_matrix(left: [f64; 6], right: [f64; 6]) -> [f64; 6] {
    [
        left[0] * right[0] + left[2] * right[1],
        left[1] * right[0] + left[3] * right[1],
        left[0] * right[2] + left[2] * right[3],
        left[1] * right[2] + left[3] * right[3],
        left[0] * right[4] + left[2] * right[5] + left[4],
        left[1] * right[4] + left[3] * right[5] + left[5],
    ]
}

fn page_dimensions(document: &Document, page_id: ObjectId) -> Option<(u32, u32)> {
    let mut current = page_id;
    let mut visited = HashSet::new();
    while visited.insert(current) {
        let dictionary = document.get_dictionary(current).ok()?;
        let page_box = dictionary
            .get(b"CropBox")
            .or_else(|_| dictionary.get(b"MediaBox"));
        if let Ok(page_box) = page_box {
            let values = page_box
                .as_array()
                .ok()?
                .iter()
                .map(object_number)
                .collect::<Option<Vec<_>>>()?;
            if values.len() == 4 {
                let width = (values[2] - values[0]).abs().ceil().max(1.0) as u32;
                let height = (values[3] - values[1]).abs().ceil().max(1.0) as u32;
                return Some((width, height));
            }
        }
        current = dictionary.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn resolve_dictionary<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    match object {
        Object::Dictionary(dictionary) => Some(dictionary),
        Object::Reference(object_id) => document.get_dictionary(*object_id).ok(),
        _ => None,
    }
}

fn resolve_stream<'a>(
    document: &'a Document,
    object: &'a Object,
) -> Option<(ObjectId, &'a Stream)> {
    let object_id = object.as_reference().ok()?;
    document
        .get_object(object_id)
        .ok()?
        .as_stream()
        .ok()
        .map(|stream| (object_id, stream))
}

fn extract_form_text(
    document: &Document,
    page_id: ObjectId,
    page_number: u32,
    form: FormPage,
) -> Result<String, String> {
    let mut synthetic = document.clone();
    let content_id = synthetic.add_object(Stream::new(dictionary! {}, form.content));
    let page = synthetic
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|error| error.to_string())?;
    page.set("Contents", Object::Reference(content_id));
    page.set("Resources", form.resources);

    match synthetic.extract_text(&[page_number]) {
        Ok(text) => Ok(text),
        Err(error) => {
            let partial = synthetic
                .extract_text_chunks(&[page_number])
                .into_iter()
                .filter_map(Result::ok)
                .collect::<String>();
            if partial.trim().is_empty() {
                Err(error.to_string())
            } else {
                Ok(partial)
            }
        }
    }
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

    #[test]
    fn expands_nested_form_text_and_preserves_horizontal_position() {
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        let left_id = add_text_form(&mut document, font_id, "LEFT_VALUE");
        let right_id = add_text_form(&mut document, font_id, "RIGHT_VALUE");
        let parent_content = lopdf::content::Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new(
                    "cm",
                    vec![1.into(), 0.into(), 0.into(), 1.into(), 50.into(), 0.into()],
                ),
                Operation::new("Do", vec![Object::Name(b"Left".to_vec())]),
                Operation::new("Q", vec![]),
                Operation::new("q", vec![]),
                Operation::new(
                    "cm",
                    vec![1.into(), 0.into(), 0.into(), 1.into(), 350.into(), 0.into()],
                ),
                Operation::new("Do", vec![Object::Name(b"Right".to_vec())]),
                Operation::new("Q", vec![]),
            ],
        }
        .encode()
        .unwrap();
        let parent_id = document.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 600.into(), 800.into()],
                "Resources" => dictionary! {
                    "XObject" => dictionary! {
                        "Left" => left_id,
                        "Right" => right_id,
                    },
                },
            },
            parent_content,
        ));
        let page_content = lopdf::content::Content {
            operations: vec![Operation::new("Do", vec![Object::Name(b"Parent".to_vec())])],
        }
        .encode()
        .unwrap();
        let content_id = document.add_object(Stream::new(dictionary! {}, page_content));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 600.into(), 800.into()],
            "Resources" => dictionary! {
                "XObject" => dictionary! { "Parent" => parent_id },
            },
            "Contents" => content_id,
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
        let path = std::env::temp_dir().join(format!(
            "invoice-extractor-nested-form-{}.pdf",
            std::process::id()
        ));
        document.save(&path).unwrap();

        let result = read_pdf_pages(&path).unwrap();
        let _ = std::fs::remove_file(path);
        assert_eq!(result.pages.len(), 1);
        assert!(result.pages[0].text.contains("LEFT_VALUE"));
        assert!(result.pages[0].text.contains("RIGHT_VALUE"));
        let positions = result.pages[0]
            .lines
            .iter()
            .map(|line| (line.words[0].text.as_str(), line.words[0].x))
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(positions.get("LEFT_VALUE"), Some(&50.0));
        assert_eq!(positions.get("RIGHT_VALUE"), Some(&350.0));
    }

    fn add_text_form(document: &mut Document, font_id: ObjectId, text: &str) -> ObjectId {
        let content = lopdf::content::Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
                Operation::new(
                    "Tj",
                    vec![Object::String(
                        text.as_bytes().to_vec(),
                        StringFormat::Literal,
                    )],
                ),
                Operation::new("ET", vec![]),
            ],
        }
        .encode()
        .unwrap();
        document.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 200.into(), 40.into()],
                "Resources" => dictionary! {
                    "Font" => dictionary! { "F1" => font_id },
                },
            },
            content,
        ))
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
