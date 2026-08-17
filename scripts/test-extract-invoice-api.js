const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');

const source = fs.readFileSync('src/ocr.js', 'utf8');
const appSource = fs.readFileSync('src/app.js', 'utf8');
const layoutSource = fs.readFileSync('src/layout.js', 'utf8');
const printSource = fs.readFileSync('src/print.js', 'utf8');
const indexSource = fs.readFileSync('src/index.html', 'utf8');
const publicApiStart = source.indexOf('/**\n * Extract all recognizable invoice fields');
const publicApiEnd = source.indexOf('// =====================================================\n// v1.7.0', publicApiStart);
assert.ok(publicApiStart >= 0 && publicApiEnd > publicApiStart, 'public API block must exist');

const calls = [];
const expected = {
  success: true,
  filePath: '/tmp/invoice.xml',
  fileName: 'invoice.xml',
  fileType: 'xml',
  pageCount: 1,
  invoices: [{ invoiceNo: '25322000000337005189' }],
  warnings: []
};
const context = {
  isTauri: true,
  invoke: async (command, args) => {
    calls.push({ command, args });
    return expected;
  },
  window: {}
};
vm.createContext(context);
vm.runInContext(source.slice(publicApiStart, publicApiEnd), context);

(async () => {
  const result = await context.extractInvoiceFile('/tmp/invoice.xml', {
    useOcr: false,
    includeRawText: false
  });
  assert.deepEqual(result, expected);
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [{
    command: 'extract_invoice_file',
    args: {
      filePath: '/tmp/invoice.xml',
      options: { useOcr: false, includeRawText: false }
    }
  }]);
  assert.equal(context.window.extractInvoiceFile, context.extractInvoiceFile);

  await assert.rejects(
    () => context.extractInvoiceFile(null),
    /请传入发票文件的绝对路径/
  );
  const directoryResult = {
    success: true,
    directoryPath: '/tmp/invoices',
    matchedFileCount: 1,
    extractedFileCount: 1,
    failedFileCount: 0,
    files: [expected],
    errors: []
  };
  context.invoke = async (command, args) => {
    calls.push({ command, args });
    return directoryResult;
  };
  const batch = await context.extractInvoiceDirectory('/tmp/invoices', { useOcr: false });
  assert.deepEqual(batch, directoryResult);
  assert.deepEqual(JSON.parse(JSON.stringify(calls.at(-1))), {
    command: 'extract_invoice_directory',
    args: {
      directoryPath: '/tmp/invoices',
      options: { useOcr: false }
    }
  });
  assert.equal(context.window.extractInvoiceDirectory, context.extractInvoiceDirectory);
  await assert.rejects(
    () => context.extractInvoiceDirectory(null),
    /请传入发票目录的绝对路径/
  );

  const directoryImportStart = appSource.indexOf('async function triggerDirectoryUpload()');
  const directoryImportEnd = appSource.indexOf('async function handleFileInput', directoryImportStart);
  assert.ok(directoryImportStart >= 0 && directoryImportEnd > directoryImportStart);
  vm.runInContext(appSource.slice(directoryImportStart, directoryImportEnd), context);

  const directoryBatch = {
    success: false,
    directoryPath: '/tmp/invoices',
    matchedFileCount: 2,
    extractedFileCount: 2,
    failedFileCount: 0,
    files: [
      { success: true, filePath: '/tmp/invoices/b.pdf', invoices: [] },
      { success: false, filePath: '/tmp/invoices/sub/a.ofd', invoices: [] }
    ],
    errors: []
  };
  const imported = [];
  context.isTauri = true;
  context.hasOcr = true;
  context.S = { feat: { ocrEnabled: true }, ocrPrecision: 'precise' };
  context.invoke = async (command) => {
    assert.equal(command, 'plugin:dialog|open');
    return '/tmp/invoices';
  };
  context.extractInvoiceDirectory = async (path, options) => {
    assert.equal(path, '/tmp/invoices');
    assert.deepEqual(JSON.parse(JSON.stringify(options)), {
      useOcr: true,
      ocrPrecision: 'precise',
      includeRawText: true
    });
    return directoryBatch;
  };
  context.processFilesIncremental = async (paths, byPath, summary) => {
    imported.push({ paths, byPath, summary });
  };
  context.toastLoading = () => {};
  context.hideToast = () => {};
  context.toast = () => {};
  context.console = console;
  await context.triggerDirectoryUpload();
  assert.deepEqual(JSON.parse(JSON.stringify(imported[0].paths)), [
    '/tmp/invoices/b.pdf',
    '/tmp/invoices/sub/a.ofd'
  ]);
  assert.equal(imported[0].byPath['/tmp/invoices/b.pdf'].success, true);
  assert.match(imported[0].summary, /已递归导入 2 个文件/);

  const mappingStart = appSource.indexOf('function findExtractedInvoice');
  const mappingEnd = appSource.indexOf('function buildPdfResults', mappingStart);
  assert.ok(mappingStart >= 0 && mappingEnd > mappingStart);
  vm.runInContext(appSource.slice(mappingStart, mappingEnd), context);
  const page = {};
  context.applyExtractedFileResult(page, {
    warnings: ['sample warning'],
    invoices: [{
      pageIndex: 1,
      source: 'pdf-text',
      invoiceNo: '26432000001658975131',
      invoiceType: 'vat-general',
      amountTax: 78.8,
      amountNoTax: 69.73,
      taxAmount: 9.07,
      taxRate: '13%',
      amountUppercase: '柒拾捌圆捌角整',
      invoiceClerk: '毛冬',
      lineItems: [{ projectName: '*其他食品*素牛筋20g', amount: 1.77 }]
    }]
  }, 1);
  assert.equal(page.invoiceNo, '26432000001658975131');
  assert.equal(page.invoiceClerk, '毛冬');
  assert.equal(page.lineItems.length, 1);
  assert.equal(page._extractionWarnings[0], 'sample warning');

  const detailsStart = appSource.indexOf('function invoiceTypeLabel');
  const detailsEnd = appSource.indexOf('function openInvModal', detailsStart);
  assert.ok(detailsStart >= 0 && detailsEnd > detailsStart);
  context.escHtml = (value) => String(value)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
  vm.runInContext(appSource.slice(detailsStart, detailsEnd), context);
  const details = context.buildExtractorDetailsHtml(page);
  assert.match(details, /毛冬/);
  assert.match(details, /柒拾捌圆捌角整/);
  assert.match(details, /\*其他食品\*素牛筋20g/);

  const dimensionsStart = layoutSource.indexOf('function getInvoiceDisplayDimensions');
  const dimensionsEnd = layoutSource.indexOf('/**\n * Calculate rotation', dimensionsStart);
  assert.ok(dimensionsStart >= 0 && dimensionsEnd > dimensionsStart);
  vm.runInContext(layoutSource.slice(dimensionsStart, dimensionsEnd), context);
  context.S = { feat: { trimWhite: true } };
  assert.deepEqual(JSON.parse(JSON.stringify(context.getInvoiceDisplayDimensions({
    ow: 1200,
    oh: 1800,
    trimmedUrl: 'data:image/png;base64,test',
    trimmedW: 1100,
    trimmedH: 700
  }))), { w: 1100, h: 700 });
  context.S.feat.trimWhite = false;
  assert.deepEqual(JSON.parse(JSON.stringify(context.getInvoiceDisplayDimensions({
    ow: 1200,
    oh: 1800,
    trimmedUrl: 'data:image/png;base64,test',
    trimmedW: 1100,
    trimmedH: 700
  }))), { w: 1200, h: 1800 });
  assert.match(indexSource, /id="marginTop"[^>]+value="2"/);
  assert.match(indexSource, /id="gapH"[^>]+value="1"/);
  assert.match(indexSource, /onclick="setCompactSpacing\(\)"/);
  assert.match(indexSource, /class="tgl on" id="toggleTrimWhite"/);
  assert.match(appSource, /trimWhite: true/);
  assert.match(appSource, /trimLayoutVersion: 1/);
  assert.match(appSource, /f\.type === 'ofd'/);
  assert.match(printSource, /spec\.crop = fileObj\.trimCrop/);

  const recordsStart = appSource.indexOf('function normalizeInvoiceNo');
  const recordsEnd = appSource.indexOf('function renderFileList', recordsStart);
  assert.ok(recordsStart >= 0 && recordsEnd > recordsStart, 'invoice record aggregation block must exist');
  const recordsContext = {};
  vm.createContext(recordsContext);
  vm.runInContext(appSource.slice(recordsStart, recordsEnd), recordsContext);
  const page1 = {
    id: 'a1', name: 'a_第1页.pdf', _pdfPath: '/tmp/a.pdf', invoiceNo: '123456', invoiceDate: '2026-08-01',
    sellerName: '销售方A', buyerName: '购买方', amountTax: 100, amountNoTax: 94.34, taxAmount: 5.66,
    lineItems: [
      { projectName: '服务费', amount: 47.17, taxAmount: 2.83 },
      { projectName: '服务费', amount: 47.17, taxAmount: 2.83 }
    ], _extractionSuccess: true, _extractionWarnings: []
  };
  const page2 = {
    id: 'a2', name: 'a_第2页.pdf', _pdfPath: '/tmp/a.pdf', invoiceNo: '', amountTax: 0,
    lineItems: [{ projectName: '补充明细', amount: 10 }], _extractionSuccess: true, _extractionWarnings: []
  };
  const duplicate = {
    id: 'b1', name: 'b.pdf', _pdfPath: '/tmp/b.pdf', invoiceNo: '123456', invoiceDate: '2026-08-02',
    sellerName: '销售方B', amountTax: 50, lineItems: [], _extractionSuccess: true, _extractionWarnings: []
  };
  const failed = {
    id: 'c1', name: 'c.pdf', _pdfPath: '/tmp/c.pdf', invoiceNo: '', amountTax: 0,
    lineItems: [], _extractionSuccess: false, _extractionWarnings: ['解析失败']
  };
  const records = recordsContext.getInvoiceRecords([page1, page2, duplicate, failed]);
  assert.equal(records.length, 3, 'two pages from one PDF must count as one invoice record');
  const mergedPdf = records.find((record) => record._sourcePath === '/tmp/a.pdf');
  assert.equal(mergedPdf._pageCount, 2);
  assert.equal(mergedPdf.lineItems.length, 3, 'legitimate duplicate rows within one page must be preserved');
  assert.equal(records.filter((record) => record._duplicateInvoice).length, 2, 'same invoice number in different files must be marked duplicate');
  assert.equal(records.find((record) => record._sourcePath === '/tmp/c.pdf')._parseStatus, 'error');
  recordsContext.getInvoiceRecords([page1, page2], false);
  assert.equal(page1._duplicateInvoice, true, 'filtered summaries must not clear global duplicate flags');
  assert.match(indexSource, /id="invoiceManagerModal"/);
  assert.match(indexSource, /data-tab="month"/);
  assert.match(indexSource, /data-tab="seller"/);
  assert.match(indexSource, /data-tab="buyer"/);
  assert.match(indexSource, /data-tab="taxRate"/);
  assert.match(indexSource, /data-tab="item"/);

  process.stdout.write('invoice extraction API, management aggregation, and compact PDF layout tests passed\n');
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
