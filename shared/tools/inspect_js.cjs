/**
 * 官方前端的结构索引器：把解包后的 readable_js 解析成 js_structure.json
 *
 * 两平台共用同一份：macOS 走 `macos/scripts/fullatlas-frontend.mjs`，
 * Windows 走 `windows/src/build.py` 与 `windows/tools/*.ps1`，
 * 以前两处各放一份 29 行的拷贝，只有 acorn 的引入路径不同，改一处忘另一处就会
 * 出现「一个平台能注入、另一个平台找不到锚点」。现在只有这一份。
 *
 * 用法：node shared/tools/inspect_js.cjs <解包根目录> <共享模块文件名>
 * 产出：<解包根目录>/js_structure.json
 *
 * acorn 是从仓库里带的（shared/tools/acorn.cjs，随 ACORN_LICENSE 分发），
 * 不依赖 node_modules —— Windows 打包机没有 npm install 也能跑。
 */
const fs = require('fs');
const path = require('path');
const acorn = require('./acorn.cjs');

const root = process.argv[2];
const sharedName = process.argv[3];
if (!root || !sharedName) {
  console.error('用法：node shared/tools/inspect_js.cjs <解包根目录> <共享模块文件名>');
  process.exit(2);
}

function parse(file) {
  const text = fs.readFileSync(file, 'utf8').replace(/\r\n/g, '\n');
  return { text, ast: acorn.parse(text, { ecmaVersion: 'latest', sourceType: 'module' }) };
}
function walk(node, visit, anc = []) {
  if (!node || typeof node !== 'object') return;
  if (node.type) visit(node, anc);
  for (const [key, value] of Object.entries(node)) {
    if (key === 'start' || key === 'end') continue;
    if (Array.isArray(value)) for (const child of value) if (child?.type) walk(child, visit, [...anc, node]); else {}
    else if (value?.type) walk(value, visit, [...anc, node]);
  }
}

const shared = parse(path.join(root, 'readable_js/assets', sharedName));
const exportMap = {};
for (const n of shared.ast.body) if (n.type === 'ExportNamedDeclaration') for (const s of n.specifiers) exportMap[s.exported.name] = s.local.name;
const result = { shared: sharedName, exports: exportMap, files: {} };

for (const name of fs.readdirSync(path.join(root, 'readable_js/assets')).filter(x => x.endsWith('.js'))) {
  const { text, ast } = parse(path.join(root, 'readable_js/assets', name));
  const meta = { imports: {}, functions: [], components: [], containers: [], roomConditions: [], cases: {}, snapshots: [] };
  for (const n of ast.body) if (n.type === 'ImportDeclaration' && n.source.value.endsWith('/' + sharedName)) for (const s of n.specifiers) if (s.type === 'ImportSpecifier') meta.imports[exportMap[s.imported.name] || s.imported.name] = s.local.name;
  walk(ast, (n, anc) => {
    if (n.type === 'FunctionDeclaration') meta.functions.push({ name: n.id.name, start: n.start, end: n.end, body: n.body.start, bodyEnd: n.body.end });
    if (n.type === 'ObjectExpression') {
      const label = n.properties.find(p => p.key?.name === '__name' && p.value.type === 'Literal');
      const setup = n.properties.find(p => p.key?.name === 'setup' && p.value.body);
      if (label && setup) {
        const declarator = [...anc].reverse().find(a => a.type === 'VariableDeclarator');
        meta.components.push({
          name: label.value.value,
          start: n.start,
          end: n.end,
          variable: declarator?.id.name,
          body: setup.value.body.start,
          bodyEnd: setup.value.body.end,
          returnStart: setup.value.body.body.filter(x => x.type === 'ReturnStatement').at(-1)?.start,
        });
      }
      if (n.properties.some(p => p.key?.name === 'streamId') && n.properties.some(p => p.key?.name === 'bagList')) {
        const fields = {};
        for (const p of n.properties) if (p.type === 'Property' && !p.computed) fields[p.key.name || p.key.value] = text.slice(p.value.start, p.value.end);
        meta.snapshots.push({ start: n.start, end: n.end, fields });
      }
    }
    if (n.type === 'CallExpression' && n.arguments?.[1]?.type === 'ObjectExpression' && n.arguments[2]?.type === 'ArrayExpression'
      && n.arguments[1].properties.some(p => p.key?.name === 'class' && p.value.value === 'panel-content')) {
      meta.containers.push({ start: n.start, end: n.end, array: n.arguments[2].start });
    }
    if (n.type === 'ConditionalExpression' && n.test.type === 'BinaryExpression' && n.test.operator === '===' && n.test.right.value === 'room') {
      meta.roomConditions.push({ start: n.start, end: n.end, test: text.slice(n.test.start, n.test.end) });
    }
    if (n.type === 'SwitchCase' && typeof n.test?.value === 'string') {
      const call = n.consequent.find(x => x.type === 'ExpressionStatement' && x.expression.type === 'CallExpression');
      if (call?.expression.callee.type === 'Identifier') meta.cases[n.test.value] = call.expression.callee.name;
    }
  });
  // acorn 的 start/end 是 UTF-16 下标，但后续注入脚本按码点算位置，这里统一换成码点偏移。
  const offsets = new Uint32Array(text.length + 1);
  let cp = 0;
  for (let i = 0; i < text.length;) {
    offsets[i] = cp;
    const width = text.codePointAt(i) > 65535 ? 2 : 1;
    if (width === 2) offsets[i + 1] = cp;
    i += width; cp++; offsets[i] = cp;
  }
  const positionKeys = new Set(['start', 'end', 'body', 'bodyEnd', 'returnStart', 'array']);
  function convert(obj) {
    if (!obj || typeof obj !== 'object') return;
    for (const [key, value] of Object.entries(obj)) {
      if (positionKeys.has(key) && typeof value === 'number') obj[key] = offsets[value];
      else if (typeof value === 'object') convert(value);
    }
  }
  convert(meta);
  result.files[name] = meta;
}

fs.writeFileSync(path.join(root, 'js_structure.json'), JSON.stringify(result, null, 2));
console.log('indexed ' + Object.keys(result.files).length + ' modules');
