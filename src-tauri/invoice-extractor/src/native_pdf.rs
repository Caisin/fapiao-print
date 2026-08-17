use crate::PdfPageRenderer;
use image::DynamicImage;
use std::path::Path;

/// PDF page renderer backed by the operating system where one is available.
///
/// macOS uses Core Graphics and therefore needs no external PDF runtime. Other
/// platforms can keep injecting an application-specific [`PdfPageRenderer`].
#[derive(Debug, Clone, Copy, Default)]
pub struct NativePdfRenderer;

impl PdfPageRenderer for NativePdfRenderer {
    fn render_page(
        &self,
        pdf_path: &Path,
        page_index: u32,
        dpi: u32,
    ) -> Result<DynamicImage, String> {
        platform::render_page(pdf_path, page_index, dpi)
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use image::{DynamicImage, RgbaImage};
    use std::ffi::c_void;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    type CGFloat = f64;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: CGFloat,
        y: CGFloat,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGSize {
        width: CGFloat,
        height: CGFloat,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGAffineTransform {
        a: CGFloat,
        b: CGFloat,
        c: CGFloat,
        d: CGFloat,
        tx: CGFloat,
        ty: CGFloat,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(
            allocator: *const c_void,
            buffer: *const u8,
            buffer_length: isize,
            is_directory: u8,
        ) -> *mut c_void;
        fn CFRelease(value: *const c_void);
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPDFDocumentCreateWithURL(url: *const c_void) -> *mut c_void;
        fn CGPDFDocumentGetNumberOfPages(document: *const c_void) -> usize;
        fn CGPDFDocumentGetPage(document: *const c_void, page_number: usize) -> *mut c_void;
        fn CGPDFPageGetBoxRect(page: *const c_void, box_type: i32) -> CGRect;
        fn CGPDFPageGetRotationAngle(page: *const c_void) -> i32;
        fn CGPDFPageGetDrawingTransform(
            page: *const c_void,
            box_type: i32,
            rect: CGRect,
            rotate: i32,
            preserve_aspect_ratio: bool,
        ) -> CGAffineTransform;
        fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
        fn CGBitmapContextCreate(
            data: *mut c_void,
            width: usize,
            height: usize,
            bits_per_component: usize,
            bytes_per_row: usize,
            color_space: *const c_void,
            bitmap_info: u32,
        ) -> *mut c_void;
        fn CGContextSetRGBFillColor(
            context: *const c_void,
            red: CGFloat,
            green: CGFloat,
            blue: CGFloat,
            alpha: CGFloat,
        );
        fn CGContextFillRect(context: *const c_void, rect: CGRect);
        fn CGContextConcatCTM(context: *const c_void, transform: CGAffineTransform);
        fn CGContextDrawPDFPage(context: *const c_void, page: *const c_void);
    }

    const MEDIA_BOX: i32 = 0;
    const CROP_BOX: i32 = 1;
    const ALPHA_PREMULTIPLIED_LAST: u32 = 1;

    struct CfHandle(*mut c_void);

    impl CfHandle {
        fn new(pointer: *mut c_void, error: impl Into<String>) -> Result<Self, String> {
            if pointer.is_null() {
                Err(error.into())
            } else {
                Ok(Self(pointer))
            }
        }
    }

    impl Drop for CfHandle {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) };
        }
    }

    pub(super) fn render_page(
        pdf_path: &Path,
        page_index: u32,
        dpi: u32,
    ) -> Result<DynamicImage, String> {
        if dpi == 0 {
            return Err("PDF 渲染 DPI 必须大于 0".to_string());
        }

        let path_bytes = pdf_path.as_os_str().as_bytes();
        let url = CfHandle::new(
            unsafe {
                CFURLCreateFromFileSystemRepresentation(
                    std::ptr::null(),
                    path_bytes.as_ptr(),
                    path_bytes.len() as isize,
                    0,
                )
            },
            "无法创建 PDF 文件 URL",
        )?;
        let document = CfHandle::new(
            unsafe { CGPDFDocumentCreateWithURL(url.0) },
            "加载 PDF 失败（文件可能受密码保护或已损坏）",
        )?;
        let page_count = unsafe { CGPDFDocumentGetNumberOfPages(document.0) };
        let page_number = page_index as usize + 1;
        if page_number > page_count {
            return Err(format!(
                "页码超出范围: 请求第{}页，PDF 共{}页",
                page_number, page_count
            ));
        }

        let page = unsafe { CGPDFDocumentGetPage(document.0, page_number) };
        if page.is_null() {
            return Err(format!("获取第{}页失败", page_number));
        }
        let mut page_box = unsafe { CGPDFPageGetBoxRect(page, CROP_BOX) };
        if page_box.size.width <= 0.0 || page_box.size.height <= 0.0 {
            page_box = unsafe { CGPDFPageGetBoxRect(page, MEDIA_BOX) };
        }
        if !page_box.size.width.is_finite()
            || !page_box.size.height.is_finite()
            || page_box.size.width <= 0.0
            || page_box.size.height <= 0.0
        {
            return Err(format!("第{}页尺寸无效", page_number));
        }

        let rotation = unsafe { CGPDFPageGetRotationAngle(page) }.rem_euclid(360);
        let (point_width, point_height) = if rotation == 90 || rotation == 270 {
            (page_box.size.height, page_box.size.width)
        } else {
            (page_box.size.width, page_box.size.height)
        };
        let pixel_scale = dpi as f64 / 72.0;
        let width = (point_width * pixel_scale).round().max(1.0) as u32;
        let height = (point_height * pixel_scale).round().max(1.0) as u32;
        let bytes_per_row = width as usize * 4;
        let buffer_len = bytes_per_row
            .checked_mul(height as usize)
            .ok_or_else(|| format!("第{}页尺寸过大", page_number))?;
        let mut pixels = vec![255u8; buffer_len];
        let color_space = CfHandle::new(
            unsafe { CGColorSpaceCreateDeviceRGB() },
            "无法创建 RGB 色彩空间",
        )?;
        let context = CfHandle::new(
            unsafe {
                CGBitmapContextCreate(
                    pixels.as_mut_ptr().cast(),
                    width as usize,
                    height as usize,
                    8,
                    bytes_per_row,
                    color_space.0,
                    ALPHA_PREMULTIPLIED_LAST,
                )
            },
            format!("创建第{}页位图失败", page_number),
        )?;
        let target = CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width: width as f64,
                height: height as f64,
            },
        };
        unsafe {
            CGContextSetRGBFillColor(context.0, 1.0, 1.0, 1.0, 1.0);
            CGContextFillRect(context.0, target);
            let transform = CGPDFPageGetDrawingTransform(page, CROP_BOX, target, 0, true);
            CGContextConcatCTM(context.0, transform);
            CGContextDrawPDFPage(context.0, page);
        }
        drop(context);

        let image = RgbaImage::from_raw(width, height, pixels)
            .ok_or_else(|| format!("读取第{}页位图失败", page_number))?;
        Ok(DynamicImage::ImageRgba8(image))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use lopdf::content::{Content, Operation};
        use lopdf::{dictionary, Document, Object, Stream};

        #[test]
        fn renders_requested_page_in_top_down_orientation() {
            let mut document = Document::with_version("1.5");
            let pages_id = document.new_object_id();
            let operations = vec![
                Operation::new("rg", vec![1.into(), 0.into(), 0.into()]),
                Operation::new("re", vec![0.into(), 100.into(), 100.into(), 100.into()]),
                Operation::new("f", vec![]),
                Operation::new("rg", vec![0.into(), 0.into(), 1.into()]),
                Operation::new("re", vec![0.into(), 0.into(), 100.into(), 100.into()]),
                Operation::new("f", vec![]),
            ];
            let content_id = document.add_object(Stream::new(
                dictionary! {},
                Content { operations }.encode().unwrap(),
            ));
            let page_id = document.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 200.into()],
                "Resources" => dictionary! {},
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
                "invoice-extractor-native-pdf-{}.pdf",
                std::process::id()
            ));
            document.save(&path).unwrap();

            let image = render_page(&path, 0, 72).unwrap().to_rgb8();
            assert_eq!((image.width(), image.height()), (100, 200));
            let top = image.get_pixel(50, 25);
            let bottom = image.get_pixel(50, 175);
            assert!(top[0] > 240 && top[2] < 15, "top pixel: {top:?}");
            assert!(
                bottom[2] > 240 && bottom[0] < 15,
                "bottom pixel: {bottom:?}"
            );
            assert!(render_page(&path, 1, 72)
                .unwrap_err()
                .contains("页码超出范围"));

            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use image::DynamicImage;
    use std::path::Path;

    pub(super) fn render_page(
        _pdf_path: &Path,
        _page_index: u32,
        _dpi: u32,
    ) -> Result<DynamicImage, String> {
        Err("当前系统没有内置 PDF 页面渲染器，请通过 PaddleOcrBackend::with_renderer 注入宿主渲染器".to_string())
    }
}
