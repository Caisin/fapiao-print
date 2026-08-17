const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');

const source = fs.readFileSync('src/ocr.js', 'utf8');
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
  process.stdout.write('extractInvoiceFile Rust IPC wrapper tests passed\n');
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
