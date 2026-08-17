const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');

const source = fs.readFileSync('src/ocr.js', 'utf8');
const appSource = fs.readFileSync('src/app.js', 'utf8');
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

  process.stdout.write('invoice extraction API and directory UI tests passed\n');
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
