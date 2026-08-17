mod ofd;
mod pdf;
mod xml;

pub(crate) use ofd::read_ofd;
pub(crate) use pdf::read_pdf_pages;
pub(crate) use xml::read_xml;

use crate::InvoiceInfo;

#[derive(Debug, Default)]
pub(crate) struct ParsedFormat {
    pub info: InvoiceInfo,
}
