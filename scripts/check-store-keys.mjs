import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';

/** Inspect syntax, so comments, regexes and nested state names cannot shadow the persistence key. */
export function inspectStore(source, file = 'store.ts') {
  const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true);
  const constants = new Map();
  const unwrap = n => n && (ts.isAsExpression(n) || ts.isParenthesizedExpression(n) || ts.isSatisfiesExpression(n)) ? unwrap(n.expression) : n;
  for (const statement of ast.statements) {
    if (ts.isVariableStatement(statement) && (statement.declarationList.flags & ts.NodeFlags.Const)) {
      for (const d of statement.declarationList.declarations) if (ts.isIdentifier(d.name)) constants.set(d.name.text, d.initializer);
    }
  }
  function literal(n, seen = new Set()) {
    n = unwrap(n);
    if (n && (ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n))) return n.text;
    if (n && ts.isIdentifier(n) && !seen.has(n.text)) return literal(constants.get(n.text), new Set([...seen, n.text]));
    return null;
  }
  const results = [];
  function walk(n) {
    if (ts.isCallExpression(n) && ts.isIdentifier(n.expression) && n.expression.text === 'persist') {
      const config = unwrap(n.arguments[1]);
      const prop = config && ts.isObjectLiteralExpression(config) ? config.properties.find(p => ts.isPropertyAssignment(p) && (ts.isIdentifier(p.name) || ts.isStringLiteral(p.name)) && p.name.text === 'name') : undefined;
      results.push({file, line:ast.getLineAndCharacterOfPosition(n.getStart(ast)).line+1, name:prop ? literal(prop.initializer) : null});
    }
    ts.forEachChild(n, walk);
  }
  walk(ast);
  return results;
}
export function checkStores(entries) {
  const seen = new Map(), errors=[];
  for (const entry of entries) {
    const where=`${entry.file}:${entry.line}`;
    if (!entry.name) errors.push(`${where}: persist() requires a statically known name`);
    else if (!entry.name.startsWith('ufop-')) errors.push(`${where}: persistence key '${entry.name}' must start with ufop-`);
    else if (seen.has(entry.name)) errors.push(`${where}: duplicate persistence key '${entry.name}' (also ${seen.get(entry.name)})`);
    else seen.set(entry.name,where);
  }
  return errors;
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
  const entries=fs.readdirSync(path.join(root,'src/stores')).filter(f=>f.endsWith('.ts')).flatMap(f=>inspectStore(fs.readFileSync(path.join(root,'src/stores',f),'utf8'),`src/stores/${f}`));
  const errors=checkStores(entries);
  if(process.argv.includes('--list'))for(const e of entries)console.log(`${e.name??'<missing>'}\t${e.file}:${e.line}`);
  for(const error of errors)console.error(`FAIL: ${error}`);
  if(!errors.length)console.log(`OK: ${entries.length} persistence keys are static, namespaced and unique`);
  process.exitCode=errors.length?1:0;
}
