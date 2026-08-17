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
  process.stdout.write('extractInvoiceFile Rust IPC wrapper tests passed\n');
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
