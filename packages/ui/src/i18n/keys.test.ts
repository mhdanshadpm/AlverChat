/**
 * The translation seam, guarded both ways.
 *
 * `text('callMute', 'Mute')` — reached through the `ns('shell')` helper the screen binds it
 * to — is two independent claims: that a key called `callMute` exists in the `shell` catalog,
 * and that the English the code carries as a fallback is the English the catalog carries.
 * Nothing in the build checks either one. i18next answers a missing key with the caller's
 * `defaultValue`, so a typo, a rename on one side only, or a reworded catalog degrades into
 * "the code's English wins silently" and the catalog rots. The seam has a second face: a
 * catalog key no call site ever asks for is a claim that the app shows a string it never
 * shows, which is how five screens came to render a "Media" tab whose tiles could not exist.
 *
 * So this file parses the real sources with the TypeScript compiler the build uses, reads
 * every call site out of the syntax tree rather than out of a regex, and checks seven ways:
 *
 *   1. no key is asked for that no catalog declares;
 *   2. no fallback differs from the value the catalog declares for that key;
 *   3. no catalog key goes unasked;
 *   4. the call sites whose key is computed rather than written stay exactly the ones they
 *      are, and each resolves against the production table the key really comes from;
 *   5. the languages the app says it serves are the languages that have catalogs on disk,
 *      and the language Settings stores has a reader;
 *   6. no English is written where it renders instead of being asked for as a key, beyond a
 *      declared table of the files that do it for a reason;
 *   7. every `{{placeholder}}` a catalog sentence asks for is passed by the call that renders
 *      it, and no variable is passed that no sentence asks for.
 *
 * Anything the reader cannot classify throws instead of being skipped: a source that does not
 * parse, an `ns()` argument that is not a literal, a key that is not a bare identifier, a call
 * with no literal English fallback, a fallback that is not a literal, an unprefixed `t()` call,
 * `useTranslation()` (whose `t` this reader cannot attribute), and an `ns()` call that is not a
 * helper binding. A guard that quietly skipped what confused it would report a clean seam for
 * the subset it happened to understand.
 */
import { describe, expect, it } from 'vitest';
import ts from 'typescript';

import chat from './en/chat';
import groups from './en/groups';
import onboarding from './en/onboarding';
import people from './en/people';
import profile from './en/profile';
import settings from './en/settings';
import shell from './en/shell';
import system from './en/system';
import { AVATAR_PREVIEW_SIZES } from '../screens/photoPipeline';
import { STATUS_LABEL } from '../screens/CallsScreen';
import { STEPS } from '../screens/OnboardingScreen';
import { PREVIEW_CONTEXT } from '../screens/ProfileScreen';
import { CORNER_LABELS } from '../screens/SettingsScreen';
import { LANGUAGES } from './index';

const CATALOGS: Record<string, Record<string, string>> = { chat, groups, onboarding, people, profile, settings, shell, system };

/** One key asked for, with the English the code carries beside it. */
type Site = { id: string; where: string; key: string; fallback: string; vars: string[] | null; varsText: string };

type Read = { sites: Site[]; dynamic: string[]; varSites: string[]; rendered: string[] };

/**
 * The call sites whose key is not written in the code that asks for it. Each names the file
 * and the expression, and the keys it can produce come from a table in that file, exported so
 * this test reads it rather than restating it. A fifth entry appearing here is the point: it
 * means someone made a label key computable, and computable keys are the one way a catalog
 * can rot without anything noticing.
 */
const DYNAMIC_SITES = [
  'screens/CallsScreen.tsx text(key)',
  'screens/OnboardingScreen.tsx text(item.labelKey)',
  'screens/ProfileScreen.tsx text(`previewUse${index}`)',
  'screens/SettingsScreen.tsx text(key)',
] as const;

/**
 * The sites whose *variables* this reader cannot read, while their key is still a literal: a
 * spread, a computed property name, or a third argument that is not an object literal. Same
 * deal as the list above — a fixed, named set, so a new one is a reviewable event rather than
 * a site the placeholder check quietly passes.
 */
const DYNAMIC_VAR_SITES: string[] = [];

/**
 * The files allowed to write English where it renders, with how many strings each writes.
 * A count, not a list, because the point is that nobody joins the table and nobody in it
 * grows: the overlay windows, the dock tooltip and the call overlay were all in here until
 * this sweep moved their 13 + 17 strings into the `shell` catalog, and they now read zero.
 *
 *   - `screens/Gallery.tsx` is the component showcase, mounted only for `?route=gallery`. Its
 *     English is fixture props handed to components ("Replying to Fathima P"), i.e. sample
 *     data, not strings the shipped app shows.
 *   - `main.tsx` says which screen name the preview harness was asked for and could not find.
 *     A developer reads that, in a window no user reaches.
 */
const RENDERED_ENGLISH: Record<string, number> = {
  'screens/Gallery.tsx': 66,
  'main.tsx': 2,
};

/** The attribute values that are read as prose because a screen reader says them out loud. */
const LABEL_ATTRIBUTES = ['aria-label', 'title', 'placeholder', 'alt'];

/** Real words, not a unit or a glyph: 'px' and 'v' render, but nobody translates them. */
const isProse = (value: string) => /[A-Za-z]/.test(value) && value.trim().length > 2;

/**
 * The English written where it renders: JSX text, the string value of a label attribute, and
 * the child of an element — following the ternaries, `&&` guards, arrays and parentheses that
 * decide which of them shows. A string that is an *argument* to a call is not a candidate,
 * because that is how a key is asked for: `text('sent', 'Send')` puts its English in the
 * catalog's hands, and `reactionTooltip(names)` builds prose somewhere this rule cannot see.
 */
function renderedOf(node: ts.Node, file: ts.SourceFile, hits: Set<string>): void {
  const collect = (expression: ts.Node | undefined): void => {
    if (!expression) return;
    if (ts.isStringLiteral(expression) || ts.isNoSubstitutionTemplateLiteral(expression)) {
      if (isProse(expression.text)) hits.add(expression.text.trim());
    } else if (ts.isTemplateExpression(expression)) {
      for (const span of [expression.head, ...expression.templateSpans.map(part => part.literal)]) {
        if (isProse(span.text)) hits.add(span.text.trim());
      }
    } else if (ts.isParenthesizedExpression(expression)) collect(expression.expression);
    else if (ts.isConditionalExpression(expression)) {
      collect(expression.whenTrue);
      collect(expression.whenFalse);
    } else if (ts.isBinaryExpression(expression) && expression.operatorToken.kind === ts.SyntaxKind.AmpersandAmpersandToken) {
      collect(expression.right);
    } else if (ts.isArrayLiteralExpression(expression)) expression.elements.forEach(collect);
  };
  const walk = (node: ts.Node): void => {
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression)) {
      const callee = node.expression.text;
      if (/^(text|t|ns)$/.test(callee) || /Text$/.test(callee)) return;
    }
    if (ts.isJsxText(node)) {
      const value = node.getText(file).trim();
      if (isProse(value)) hits.add(value);
    } else if (ts.isJsxAttribute(node) && node.initializer && LABEL_ATTRIBUTES.includes(node.name.getText(file))) {
      collect(ts.isJsxExpression(node.initializer) ? node.initializer.expression : node.initializer);
    } else if (ts.isJsxExpression(node) && node.expression && (ts.isJsxElement(node.parent) || ts.isJsxFragment(node.parent))) {
      collect(node.expression);
    }
    ts.forEachChild(node, walk);
  };
  walk(node);
}

/** What each catalog-less string in one file is, and what the table says it should be. */
function renderedViolations(counts: Record<string, number>): string[] {
  const wrong: string[] = [];
  for (const [file, found] of Object.entries(counts)) {
    const allowed = RENDERED_ENGLISH[file];
    if (allowed === undefined) wrong.push(`${file}: ${found} strings rendered with no key asked for, and the file is not in the table`);
    else if (found !== allowed) wrong.push(`${file}: ${found} strings rendered but the table declares ${allowed}`);
  }
  for (const file of Object.keys(RENDERED_ENGLISH)) if (counts[file] === undefined) wrong.push(`${file}: in the table but rendering no English — remove it from the table`);
  return wrong.sort();
}

/** What each dynamic site can ask for, straight off the production tables. */
function pairsFromTables(): Record<string, { key: string; fallback: string }[]> {
  return {
    shell: Object.values(STATUS_LABEL).map(([key, fallback]) => ({ key, fallback })),
    onboarding: STEPS.map(step => ({ key: step.labelKey, fallback: step.label })),
    settings: Object.values(CORNER_LABELS).map(([key, fallback]) => ({ key, fallback })),
    profile: PREVIEW_CONTEXT.map((fallback, index) => ({ key: `previewUse${index}`, fallback })),
  };
}

/** The helpers one file binds: `const text = ns('shell')`, memoized or not. */
function helpersOf(file: ts.SourceFile, names: Set<string>): Map<string, string> {
  const helpers = new Map<string, string>();
  const bindings = (node: ts.Node): void => {
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.initializer) {
      const call = unwrapMemo(node.initializer);
      if (call && isNsCall(call) && names.has('ns')) {
        const namespace = literalOf(call.arguments[0]);
        if (namespace === null) throw new Error(`${file.fileName}: ns() with a namespace that is not a literal`);
        const known = helpers.get(node.name.text);
        if (known && known !== namespace) {
          throw new Error(`${file.fileName}: the helper ${node.name.text} is bound to both ${known} and ${namespace}`);
        }
        helpers.set(node.name.text, namespace);
      }
    }
    ts.forEachChild(node, bindings);
  };
  bindings(file);
  return helpers;
}

/** `useMemo(() => ns('shell'), [])` reads back to the same call as `ns('shell')`. */
function unwrapMemo(node: ts.Expression): ts.CallExpression | null {
  if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === 'ns') return node;
  if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === 'useMemo') {
    const [factory] = node.arguments;
    if (factory && ts.isArrowFunction(factory) && ts.isCallExpression(factory.body)) return factory.body;
  }
  return null;
}

const isNsCall = (call: ts.CallExpression) => ts.isIdentifier(call.expression) && call.expression.text === 'ns';

function literalOf(node: ts.Node | undefined): string | null {
  if (!node) return null;
  if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) return node.text;
  return null;
}

/**
 * The variables a call site hands the sentence: `{ count, name: peer.name }` supplies `count`
 * and `name`. A spread, a computed key, or a third argument that is not an object literal is
 * not readable here, so it comes back as `null` and lands in `DYNAMIC_VAR_SITES` — the same
 * treatment a computed *key* gets, and for the same reason: unreadable must not mean uncounted.
 */
function varsOf(node: ts.Node | undefined): { keys: string[] | null; text: string } {
  if (!node) return { keys: [], text: '' };
  const source = node.getText().replace(/\s+/g, ' ').trim();
  if (!ts.isObjectLiteralExpression(node)) return { keys: null, text: source };
  const keys: string[] = [];
  for (const property of node.properties) {
    if (ts.isShorthandPropertyAssignment(property)) keys.push(property.name.text);
    else if (ts.isPropertyAssignment(property) && !ts.isComputedPropertyName(property.name)) {
      keys.push(property.name.getText().replace(/^['"]|['"]$/g, ''));
    } else return { keys: null, text: source };
  }
  return { keys, text: source };
}

/** The placeholders i18next will substitute into a sentence, `{{count, number}}` included. */
function tokensOf(sentence: string): string[] {
  return [...sentence.matchAll(/\{\{\s*([A-Za-z0-9_]+)\s*(?:,[^}]*)?\}\}/g)].map(match => match[1]);
}

/**
 * The names this reader treats as the seam. Everything imported from the i18n module counts,
 * and `useTranslation` counts from wherever it comes: its `t` takes a key this reader could
 * never attribute to a namespace, so a file that reaches for it is a file the guard cannot read.
 */
function i18nNames(file: ts.SourceFile): Set<string> {
  const names = new Set<string>();
  for (const statement of file.statements) {
    if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier)) continue;
    const bindings = statement.importClause?.namedBindings;
    if (!bindings || !ts.isNamedImports(bindings)) continue;
    const fromI18n = /(^|\/)i18n$/.test(statement.moduleSpecifier.text);
    for (const element of bindings.elements) {
      if (fromI18n || element.name.text === 'useTranslation') names.add(element.name.text);
    }
  }
  return names;
}

/** Every `ns(` in the file is a helper binding or this reader has lost track of the seam. */
function assertAllNsBound(file: ts.SourceFile, helpers: Map<string, string>): void {
  let calls = 0;
  const walk = (node: ts.Node): void => {
    if (ts.isCallExpression(node) && isNsCall(node)) calls += 1;
    ts.forEachChild(node, walk);
  }
  walk(file);
  if (calls !== helpers.size) {
    throw new Error(
      `${file.fileName}: ${calls} ns() calls but ${helpers.size} helper bindings — an ns() call is not a helper this reader can attribute`,
    );
  }
}

function read(file: string, text: string): Read {
  const kind = file.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS;
  const parsed = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, kind);
  // createSourceFile does not throw on a syntax error; it returns a tree with holes in it, and
  // a file that half-parsed is exactly the source that would report a clean seam by omission.
  // `parseDiagnostics` is on the parser's file object but not on the public SourceFile type.
  const { parseDiagnostics } = parsed as ts.SourceFile & { parseDiagnostics?: ts.Diagnostic[] };
  const [diagnostic] = parseDiagnostics ?? [];
  if (diagnostic) {
    throw new Error(
      `${file}: the source does not parse (${ts.flattenDiagnosticMessageText(diagnostic.messageText, ' ')}) — this guard reads the seam out of the syntax tree, and a tree with holes in it reads as a seam with none`,
    );
  }
  const names = i18nNames(parsed);
  if (names.has('useTranslation')) throw new Error(`${file}: useTranslation() is a seam this reader does not attribute`);
  const helpers = helpersOf(parsed, names);
  assertAllNsBound(parsed, helpers);
  const sites: Site[] = [];
  const dynamic: string[] = [];
  const varSites: string[] = [];
  const walk = (node: ts.Node): void => {
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression)) {
      const callee = node.expression.text;
      const namespace = helpers.get(callee);
      if (namespace && callee !== 'ns') {
        const where = `${file}:${ts.getLineAndCharacterOfPosition(parsed, node.getStart()).line + 1}`;
        const [keyNode, fallbackNode] = node.arguments;
        const key = literalOf(keyNode);
        if (key === null) {
          dynamic.push(`${file} ${callee}(${keyNode?.getText(parsed).replace(/\s+/g, ' ').trim() ?? ''})`);
        } else {
          if (!/^[A-Za-z0-9_]+$/.test(key)) throw new Error(`${where}: the key ${JSON.stringify(key)} is not a bare identifier`);
          const fallback = literalOf(fallbackNode);
          if (fallback === null) {
            throw new Error(
              `${where}: ${callee}(${JSON.stringify(key)}) has no literal English fallback beside it — the catalog is the only copy`,
            );
          }
          const vars = varsOf(node.arguments[2]);
          if (vars.keys === null) varSites.push(`${file} ${callee}('${key}') ${vars.text}`);
          sites.push({ id: `${namespace}:${key}`, where, key, fallback, vars: vars.keys, varsText: vars.text });
        }
      } else if (names.has('t') && callee === 't') {
        throw new Error(`${file}: a t() call asks for a key with no namespace; every key is asked for as ns:key`);
      }
    }
    ts.forEachChild(node, walk);
  }
  walk(parsed);
  const rendered = new Set<string>();
  renderedOf(parsed, parsed, rendered);
  return { sites, dynamic, varSites, rendered: [...rendered] };
}

/**
 * The sources the bundle is built from, as raw text. Vite's own glob gives this test the same
 * bytes the compiler sees, so the seam is read off the real tree rather than a list of files
 * someone maintains beside it.
 */
const RAW = import.meta.glob('../**/*.{ts,tsx}', { eager: true, import: 'default', query: '?raw' }) as Record<string, string>;

/**
 * The language directories that actually carry catalogs, read off the same glob. Vite spells a
 * file beside this one `./en/chat.ts` and a file elsewhere `../i18n/en/chat.ts`, so both
 * spellings mean the same directory and both are accepted.
 */
const CATALOG_DIRECTORIES = [...new Set(Object.keys(RAW).map(path => /(?:^\.\/|^\.\.\/i18n\/)([a-z]{2})\//.exec(path)?.[1]).filter((dir): dir is string => dir !== undefined))].sort();

function tree(): Read & { renderedByFile: Record<string, number> } {
  const out: Read & { renderedByFile: Record<string, number> } = { sites: [], dynamic: [], varSites: [], rendered: [], renderedByFile: {} };
  for (const path of Object.keys(RAW).sort()) {
    const base = path.slice('../'.length);
    // The catalogs are the seam's other side, and this guard's own file is not a call site.
    if (base.startsWith('i18n/') || /\.test\./.test(base)) continue;
    const found = read(base, RAW[path]);
    out.sites.push(...found.sites);
    out.dynamic.push(...found.dynamic);
    out.varSites.push(...found.varSites);
    if (found.rendered.length) out.renderedByFile[base] = found.rendered.length;
  }
  return out;
}

/**
 * Direction seven. A sentence in a catalog is a small program: `{{count}}` is a parameter, and
 * the call site is the only place that can supply it. When the two disagree the user sees the
 * literal token or a hole where the number was, in one language only, and every one of the six
 * directions above stays green because the key exists, both sides agree, and nothing is unused.
 */
function interpolationViolations(sites: Site[]): string[] {
  const wrong: string[] = [];
  for (const site of sites) {
    if (site.vars === null) continue;
    const sentence = CATALOGS[site.id.split(':')[0]]?.[site.key];
    if (sentence === undefined) continue;
    const needed = tokensOf(sentence);
    for (const token of needed) {
      if (!site.vars.includes(token)) {
        wrong.push(`${site.where} ${site.id} substitutes {{${token}}} and the call passes ${site.vars.length ? site.vars.join(', ') : 'nothing'}`);
      }
    }
    for (const name of site.vars) {
      if (!needed.includes(name)) wrong.push(`${site.where} ${site.id} passes ${name}, which the sentence never asks for`);
    }
  }
  return wrong;
}

function missingFrom(catalogs: Record<string, Record<string, string>>, sites: Site[]): string[] {
  return sites.filter(site => !Object.prototype.hasOwnProperty.call(catalogs[site.id.split(':')[0]] ?? {}, site.key)).map(site => `${site.where} ${site.id}`);
}

/**
 * Direction eight. A limit the code enforces has to be handed to the sentence that states it,
 * because a number typed into copy is a second rule that nobody runs: `groups:uploadHint` said
 * "256 px" beside a `photoPipeline.ts` constant that decided the size, and the sentence would
 * have kept saying 256 on screen after the constant moved. A magnitude written into the copy is
 * refused; a magnitude the call passes in as a variable is the same words with one owner.
 */
const MAGNITUDE = /\b\d+(?:[.,]\d+)?\s*(?:px|KB|MB|GB|TB|bytes|characters?|seconds?|minutes?|hours?|days?)\b/;

function typedMagnitudes(catalogs: Record<string, Record<string, string>>, sites: Site[]): string[] {
  const found: string[] = [];
  for (const [namespace, catalog] of Object.entries(catalogs)) {
    for (const [key, value] of Object.entries(catalog)) {
      if (MAGNITUDE.test(value)) found.push(`en/${namespace}.ts ${key} :: ${JSON.stringify(value)}`);
    }
  }
  for (const site of sites) {
    if (MAGNITUDE.test(site.fallback)) found.push(`${site.where} ${site.id} :: ${JSON.stringify(site.fallback)}`);
  }
  return found;
}

function divergingFrom(catalogs: Record<string, Record<string, string>>, sites: Site[]): string[] {
  return sites
    .filter(site => catalogs[site.id.split(':')[0]]?.[site.key] !== undefined)
    .filter(site => catalogs[site.id.split(':')[0]][site.key] !== site.fallback)
    .map(site => `${site.where} ${site.id} :: catalog ${JSON.stringify(catalogs[site.id.split(':')[0]][site.key])} vs code ${JSON.stringify(site.fallback)}`);
}

function unreadFrom(catalogs: Record<string, Record<string, string>>, asked: Set<string>): string[] {
  const unread: string[] = [];
  for (const [namespace, catalog] of Object.entries(catalogs)) {
    for (const key of Object.keys(catalog)) if (!asked.has(`${namespace}:${key}`)) unread.push(`${namespace}:${key}`);
  }
  return unread;
}

describe('the translation key seam', () => {
  const found = tree();
  const tables = pairsFromTables();
  const asked = new Set(found.sites.map(site => site.id));
  for (const [namespace, pairs] of Object.entries(tables)) {
    for (const pair of pairs) asked.add(`${namespace}:${pair.key}`);
  }

  it('sees a seam worth guarding', () => {
    expect(found.sites.length).toBeGreaterThan(1000);
    expect(asked.size).toBeGreaterThan(900);
    expect(Object.values(CATALOGS).reduce((total, catalog) => total + Object.keys(catalog).length, 0)).toBeGreaterThan(900);
    // Direction seven would be vacuous on a seam with no interpolation: 147 of the 1150 literal
    // asks hand the sentence variables, and the catalogs declare 155 placeholders.
    expect(found.sites.filter(site => site.vars && site.vars.length > 0).length).toBeGreaterThan(140);
    expect(Object.values(CATALOGS).reduce((total, catalog) => total + Object.values(catalog).reduce((n, value) => n + tokensOf(value).length, 0), 0)).toBeGreaterThan(150);
  });

  it('is only crossed dynamically where a production table says so', () => {
    expect(found.dynamic.slice().sort()).toEqual([...DYNAMIC_SITES].sort());
  });

  it('asks for nothing no catalog declares', () => {
    expect(missingFrom(CATALOGS, found.sites)).toEqual([]);
  });

  it('carries the same English on both sides', () => {
    expect(divergingFrom(CATALOGS, found.sites)).toEqual([]);
  });

  it('declares nothing the app never asks for', () => {
    expect(unreadFrom(CATALOGS, asked)).toEqual([]);
  });

  // Direction six. The five above only speak about strings that already ask for a key, so a
  // label written straight into JSX passes all of them while staying English forever in every
  // language. The overlay windows carried 17 of these and the dock tooltip 6 more, which is
  // what the sweep removed; what is left is the table above.
  it('renders no English that did not go through a key', () => {
    expect(renderedViolations(found.renderedByFile)).toEqual([]);
  });

  it('supplies every placeholder the sentence asks for', () => {
    expect(interpolationViolations(found.sites)).toEqual([]);
  });

  it('leaves every enforced limit to the code that enforces it', () => {
    expect(typedMagnitudes(CATALOGS, found.sites)).toEqual([]);
  });

  // The teeth of direction eight: the number is caught in both copies of the sentence, and the
  // same words with the magnitude passed in are clean.
  it('catches a magnitude typed into either side of the seam', () => {
    const vars = (body: string) => read('Scratch.tsx', `import { ns } from '../i18n';\n${body}`).sites;
    const copy = (value: string) => vars(`const text = ns('groups');\ntext('uploadHint', ${JSON.stringify(value)});`);

    expect(typedMagnitudes({}, copy(`Pictures are resized to 256 px.`))).toEqual([
      'Scratch.tsx:3 groups:uploadHint :: "Pictures are resized to 256 px."',
    ]);
    expect(typedMagnitudes({}, copy(`Pictures are resized to {{px}} px.`))).toEqual([]);
    expect(typedMagnitudes({ groups: { uploadHint: 'Pictures are resized to 256 px.' } }, [])).toEqual([
      'en/groups.ts uploadHint :: "Pictures are resized to 256 px."',
    ]);
    expect(typedMagnitudes({ groups: { errorSize: 'That picture is bigger than 10 MB.' } }, [])).toEqual([
      'en/groups.ts errorSize :: "That picture is bigger than 10 MB."',
    ]);
    expect(typedMagnitudes({ groups: { descriptionHelper: 'Up to 140 characters.' } }, [])).toEqual([
      'en/groups.ts descriptionHelper :: "Up to 140 characters."',
    ]);
    expect(typedMagnitudes({ settings: { editDay: '24 hours' } }, [])).toEqual(['en/settings.ts editDay :: "24 hours"']);
    // A number that is not a magnitude is not this rule's business, and neither is a number the
    // call passes as a value: the sample toast renders 2.4 MB from an argument, not from copy.
    expect(typedMagnitudes({ people: { discoveredOne: 'Found 1 person on this network' } }, [])).toEqual([]);
    expect(typedMagnitudes({}, vars(`const text = ns('system');\ntext('toastUploadedBody', '{{name}} ({{size}})', { name, size: '2.4 MB' });`))).toEqual([]);
  });

  // The teeth of direction seven, on the real `chat:downloadFile` ('Download {{name}}').
  it('catches a hole where a sentence asked for a variable, and a variable nothing asked for', () => {
    const vars = (body: string) => read('Scratch.tsx', `import { ns } from '../i18n';\n${body}`).sites;
    const violations = (body: string) => interpolationViolations(vars(body));

    expect(violations(`const text = ns('chat');\ntext('downloadFile', 'Download {{name}}');`)).toEqual([
      'Scratch.tsx:3 chat:downloadFile substitutes {{name}} and the call passes nothing',
    ]);
    expect(violations(`const text = ns('chat');\ntext('downloadFile', 'Download {{name}}', { name });`)).toEqual([]);
    expect(violations(`const text = ns('chat');\ntext('downloadFile', 'Download {{name}}', { name: peer.name });`)).toEqual([]);
    expect(violations(`const text = ns('chat');\ntext('downloadFile', 'Download {{name}}', { fileName });`)).toEqual([
      'Scratch.tsx:3 chat:downloadFile substitutes {{name}} and the call passes fileName',
      'Scratch.tsx:3 chat:downloadFile passes fileName, which the sentence never asks for',
    ]);
    // A spread is not readable here, so it is named rather than passed — the same deal as a
    // computed key in DYNAMIC_SITES.
    const spread = vars(`const text = ns('chat');\ntext('downloadFile', 'Download {{name}}', { ...extra });`);
    expect(interpolationViolations(spread)).toEqual([]);
    expect(read('Scratch.tsx', `import { ns } from '../i18n';\nconst text = ns('chat');\ntext('downloadFile', 'Download {{name}}', { ...extra });`).varSites).toEqual([
      "Scratch.tsx text('downloadFile') { ...extra }",
    ]);
    // A sentence with no placeholder and no variables is clean, and one extra var is not.
    expect(violations(`const text = ns('chat');\ntext('retry', 'Retry');`)).toEqual([]);
    expect(violations(`const text = ns('chat');\ntext('retry', 'Retry', { name });`)).toEqual([
      'Scratch.tsx:3 chat:retry passes name, which the sentence never asks for',
    ]);
  });

  it('reads the variables at every site that passes them', () => {
    expect(found.varSites.slice().sort()).toEqual([...DYNAMIC_VAR_SITES].sort());
  });

  // The teeth of the new direction: the shapes a hardcoded label actually arrives in.
  it('reads hardcoded English out of every position it renders in', () => {
    const seen = (body: string) => read('Scratch.tsx', body).rendered;
    const header = `import { ns } from '../i18n';\n`;

    expect(seen(`const a = <button aria-label="Send it now" />;`)).toEqual(['Send it now']);
    expect(seen(`const a = <input placeholder="Write a message" title="Open the thread" alt="Avatar of the sender" />;`)).toEqual([
      'Write a message',
      'Open the thread',
      'Avatar of the sender',
    ]);
    expect(seen(`const a = <span>Nothing here yet</span>;`)).toEqual(['Nothing here yet']);
    expect(seen(`const a = <b>{online ? 'One peer found' : 'Searching the local network'}</b>;`)).toEqual([
      'One peer found',
      'Searching the local network',
    ]);
    expect(seen(`const a = <div>{['Pin this message', 'Unpin it']}</div>;`)).toEqual(['Pin this message', 'Unpin it']);
    expect(seen(`const a = <p>{isRinging ? <>Incoming call from {' '}</> : ''}</p>;`)).toEqual(['Incoming call from']);
    expect(seen("const a = <b>{`Replying to ${name}`}</b>;")).toEqual(['Replying to']);

    // An ask is not a candidate, and neither is prose built inside a call.
    expect(seen(`${header}const text = ns('chat');\nconst a = <b>{text('sent', 'Send it now')}</b>;`)).toEqual([]);
    expect(seen(`${header}const a = <b>{tooltip('Send it now')}</b>;`)).toEqual([]);
    expect(seen(`${header}const a = <b>{'px'}</b>;`)).toEqual([]);
    expect(seen(`${header}const a = <b>{"v"}</b>;`)).toEqual([]);
  });

  // And the table has edges: a file joining it, a file in it growing, and a file in it
  // cleaning itself out all fail, so the allowlist cannot quietly become the whole codebase.
  it('refuses a hardcoded-English file that is not in the table, and a table that goes stale', () => {
    const clean = { ...RENDERED_ENGLISH };
    expect(renderedViolations(clean)).toEqual([]);
    expect(renderedViolations({ ...clean, 'screens/SettingsScreen.tsx': 1 })).toEqual([
      'screens/SettingsScreen.tsx: 1 strings rendered with no key asked for, and the file is not in the table',
    ]);
    expect(renderedViolations({ ...clean, 'main.tsx': 3 })).toEqual(['main.tsx: 3 strings rendered but the table declares 2']);
    expect(renderedViolations({ 'main.tsx': 2 })).toEqual([
      'screens/Gallery.tsx: in the table but rendering no English — remove it from the table',
    ]);
  });

  it('resolves every dynamic table against the catalog it lands in', () => {
    expect(PREVIEW_CONTEXT).toHaveLength(AVATAR_PREVIEW_SIZES.length);
    const wrong: string[] = [];
    for (const [namespace, pairs] of Object.entries(tables)) {
      for (const pair of pairs) {
        const declared = CATALOGS[namespace]?.[pair.key];
        // Direction seven cannot check a computed key's sentence: a table says which key and
        // which English, never which variables its caller will pass. Asserting that none of the
        // sentences the four tables produce has a placeholder keeps that gap provable rather
        // than observed — a table that grows one has to be handed its parameters on purpose.
        for (const source of [declared ?? '', pair.fallback]) {
          if (tokensOf(source).length) wrong.push(`${namespace}:${pair.key} substitutes ${tokensOf(source).join(', ')} from a computed site`);
        }
        if (declared === undefined) wrong.push(`${namespace}:${pair.key} is produced but never declared`);
        else if (declared !== pair.fallback) {
          wrong.push(`${namespace}:${pair.key} :: catalog ${JSON.stringify(declared)} vs table ${JSON.stringify(pair.fallback)}`);
        }
      }
    }
    expect(wrong).toEqual([]);
  });

  it('knows the eight namespaces the screens register', () => {
    expect(Object.keys(CATALOGS).sort()).toEqual(['chat', 'groups', 'onboarding', 'people', 'profile', 'settings', 'shell', 'system']);
  });

  // The language Settings stores used to be read by nothing, and the picker offered a value no
  // catalog could answer. Both halves now come from one list, and that list has to match the
  // directories on disk: a language with no strings beside it would fall back to English
  // silently, which is the same silence this file exists to end.
  it('serves exactly the languages it has catalogs for', () => {
    expect(LANGUAGES.slice().sort()).toEqual(CATALOG_DIRECTORIES);
  });

  // `settings.language` was written by the picker and read by nothing, which is the same
  // failure item 36 recorded for an event and item 38 for a command: a stored value with no
  // reader is a setting that pretends to work. These two scans are the cheapest shape of
  // "something reads it" that does not need a second catalog to be true.
  it('reads the language it stores, and offers the list it serves', () => {
    const outside = Object.entries(RAW).filter(([path]) => !/\.test\./.test(path) && !path.startsWith('./'));
    const readers = outside.filter(([, source]) => source.includes('setLanguage(')).map(([path]) => path).sort();
    expect(readers).toEqual(['../screens/shared.tsx']);
    expect(outside.filter(([, source]) => source.includes('LANGUAGES')).map(([path]) => path).sort()).toEqual(['../screens/SettingsScreen.tsx']);
  });

  // The teeth: each drift mode the four directions above exist to catch, run through the same
  // reader over a source string, so a fix that only satisfies one spelling fails here.
  it('catches each drift mode on a source it has never seen', () => {
    const catalogs = { shell: { callMute: 'Mute' }, chat: { sent: 'Send' } };
    const header = `import { ns } from '../i18n';\n`;
    const one = (body: string) => read('Scratch.tsx', header + body).sites;

    expect(missingFrom(catalogs, one(`const text = ns('shell');\ntext('callUnmute', 'Unmute');`))).toEqual(['Scratch.tsx:3 shell:callUnmute']);
    expect(divergingFrom(catalogs, one(`const text = ns('shell');\ntext('callMute', 'Mute the microphone');`))).toEqual([
      'Scratch.tsx:3 shell:callMute :: catalog "Mute" vs code "Mute the microphone"',
    ]);
    expect(divergingFrom(catalogs, one(`const text = ns('shell');\ntext('callMute', 'Mute');`))).toEqual([]);
    expect(unreadFrom(catalogs, new Set(['chat:sent']))).toEqual(['shell:callMute']);

    // The same key, spelled once with an ellipsis escape and once with the character, agrees.
    const escaped = one(`const text = ns('chat');\ntext('sent', 'Send\\u2026');`);
    const plain = one(`const text = ns('chat');\ntext('sent', 'Send…');`);
    expect(escaped[0].fallback).toBe(plain[0].fallback);
  });

  it('refuses a source it cannot classify rather than skipping it', () => {
    const header = `import { ns } from '../i18n';\n`;
    const cases: [string, string][] = [
      [`const text = ns(name);\ntext('a', 'A');`, 'not a literal'],
      [`const text = ns('shell');\ntext('a');`, 'no literal English fallback'],
      [`const text = ns('shell');\ntext('a', name);`, 'no literal English fallback'],
      [`const text = ns('shell');\ntext('a.b', 'A');`, 'bare identifier'],
      [`import { t } from '../i18n';\nt('a', 'A');`, 'no namespace'],
      [`import { useTranslation } from 'react-i18next';\nconst { t } = useTranslation();`, 'useTranslation'],
      [`ns('shell')('a', 'A');`, 'is not a helper'],
      [`const text = ns('shell');\nconst other = ns(name);\ntext('a', 'A');`, 'not a literal'],
    ];
    for (const [body, phrase] of cases) {
      expect(() => read('Scratch.tsx', header + body), body).toThrow(phrase);
    }
    // A file with a stray token in it still produces a tree, and the tree is missing whatever
    // the parser could not read. This session introduced exactly one such file while sweeping
    // the overlay windows and every direction above stayed green on it.
    expect(() => read('Scratch.tsx', `const a = <button type="button"}}>Open it</button>;`)).toThrow('the source does not parse');
  });

  it('reads the four dynamic tables as the call sites that use them', () => {
    const synthetic = (name: string, body: string) => read(name, `import { ns, registerNamespace } from '../i18n';\n${body}`).dynamic;
    expect(synthetic('screens/CallsScreen.tsx', `const text = ns('shell');\ntext(key, fallback);`)).toEqual(['screens/CallsScreen.tsx text(key)']);
    expect(synthetic('screens/ProfileScreen.tsx', 'const text = ns(\'profile\');\ntext(`previewUse${index}`, PREVIEW_CONTEXT[index]);')).toEqual([
      'screens/ProfileScreen.tsx text(`previewUse${index}`)',
    ]);
  });
});
