// Presentation-only GRL/JSON tooling for the editable examples.
//
// The engine's own parser stays authoritative for everything that affects a
// result: `validate_rule` reports whether GRL is loadable, `register_rule`
// parses the source that actually executes, and a missing embedding is
// discovered from the engine's own error rather than guessed here. These
// helpers only colour text, slice the source into per-rule spans for the trace
// view, and pre-resolve the vectors a run is likely to need. A miss changes how
// a rule reads on screen, or costs one extra resolve round-trip — never what
// runs.

const AMP = /[&<>]/g;
const ENTITY = { '&': '&amp;', '<': '&lt;', '>': '&gt;' };

function escapeHtml(text) {
  return text.replace(AMP, (c) => ENTITY[c]);
}

// --- structural scanning ----------------------------------------------------

/**
 * A copy of `source` with string bodies and comments blanked to spaces, so
 * brace matching and keyword search never trip over their contents. Newlines
 * survive, keeping every index and line number aligned with the original.
 */
function mask(source) {
  const out = new Array(source.length);
  let i = 0;
  const blank = (from, to) => {
    for (let k = from; k < to; k++) out[k] = source[k] === '\n' ? '\n' : ' ';
  };
  while (i < source.length) {
    const ch = source[i];
    if (ch === '"') {
      let j = i + 1;
      while (j < source.length && source[j] !== '"') j += source[j] === '\\' ? 2 : 1;
      j = Math.min(j + 1, source.length);
      blank(i, j);
      i = j;
    } else if (ch === '/' && source[i + 1] === '/') {
      let j = source.indexOf('\n', i);
      if (j < 0) j = source.length;
      blank(i, j);
      i = j;
    } else {
      out[i] = ch;
      i++;
    }
  }
  return out.join('');
}

/** Index of the bracket closing the one at `open`, or -1. Operates on masked text. */
function matchBracket(masked, open) {
  const closer = masked[open] === '(' ? ')' : '}';
  let depth = 0;
  for (let i = open; i < masked.length; i++) {
    if (masked[i] === masked[open]) depth++;
    else if (masked[i] === closer && --depth === 0) return i;
  }
  return -1;
}

function unquote(literal) {
  try {
    return JSON.parse(literal);
  } catch {
    return literal.slice(1, -1);
  }
}

const FACT_PATH = /\b[A-Z][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*\b/g;
const STRING_LITERAL = /"(?:[^"\\]|\\.)*"/g;
const VECTOR_CALL = /\b[scbm]_[A-Za-z_][A-Za-z0-9_]*\s*\(/g;

/**
 * Split GRL source into its rules, keeping each rule's `when` and `then` text
 * verbatim so the trace can show what the engine was handed rather than a
 * hand-written paraphrase of it.
 */
export function splitRules(source) {
  const masked = mask(source);
  const rules = [];
  const heads = /\brule\b/g;
  let head;
  while ((head = heads.exec(masked))) {
    const open = masked.indexOf('{', head.index);
    if (open < 0) break;
    const close = matchBracket(masked, open);
    if (close < 0) break;

    const header = source.slice(head.index, open);
    const body = source.slice(open + 1, close);
    const bodyMask = masked.slice(open + 1, close);
    const whenAt = bodyMask.search(/\bwhen\b/);
    const thenAt = bodyMask.search(/\bthen\b/);
    const salience = header.match(/\bsalience\s+(-?\d+)/);

    rules.push({
      name: unquote((header.match(/"(?:[^"\\]|\\.)*"/) || ['""'])[0]),
      salience: salience ? Number(salience[1]) : null,
      noLoop: /\bno-loop\b/.test(header),
      when: whenAt < 0 ? '' : body.slice(whenAt + 4, thenAt < 0 ? undefined : thenAt).trim(),
      then: thenAt < 0 ? '' : body.slice(thenAt + 4).trim(),
      source: source.slice(head.index, close + 1)
    });
    heads.lastIndex = close + 1;
  }
  return rules;
}

/** Fact paths a rule's `when` reads. */
export function pathsRead(rule) {
  return [...new Set(rule.when.match(FACT_PATH) || [])];
}

/** Fact paths a rule's `then` assigns, in source order. */
export function pathsAssigned(rule) {
  const assigns = rule.then.match(/\b[A-Z][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*(?=\s*=[^=])/g) || [];
  return [...new Set(assigns)];
}

/**
 * The texts a run will need vectors for: string literals passed to a vector
 * function, plus the fact paths whose *values* are passed to one. Lisp operator
 * tokens (`v:add`, `c:…`) are syntax, not text to embed, so they are excluded.
 *
 * This is a pre-resolve pass, not a semantic authority: whatever it misses the
 * engine reports as a missing embedding, and the caller resolves it and retries.
 */
export function vectorInputs(source) {
  const masked = mask(source);
  const literals = new Set();
  const paths = new Set();
  let call;
  VECTOR_CALL.lastIndex = 0;
  while ((call = VECTOR_CALL.exec(masked))) {
    const open = VECTOR_CALL.lastIndex - 1;
    const close = matchBracket(masked, open);
    // Rescan from just inside the call so nested vector calls are found too.
    VECTOR_CALL.lastIndex = open + 1;
    if (close < 0) continue;
    const args = source.slice(open + 1, close);
    for (const literal of args.match(STRING_LITERAL) || []) {
      const text = unquote(literal);
      if (!/^[vc]:/.test(text)) literals.add(text);
    }
    for (const path of args.match(FACT_PATH) || []) paths.add(path);
  }
  return { literals: [...literals], paths: [...paths] };
}

// --- highlighting -----------------------------------------------------------

const GRL_TOKENS = [
  ['comment', /^\/\/[^\n]*/],
  ['vop', /^"[vc]:(?:[^"\\]|\\.)*"/],
  ['string', /^"(?:[^"\\]|\\.)*"/],
  ['number', /^-?\d+(?:\.\d+)?/],
  ['keyword', /^(?:rule|when|then|salience|no-loop|retract|insert|update|true|false|null)\b/],
  ['fn', /^[scbm]_[A-Za-z_][A-Za-z0-9_]*\b/],
  ['path', /^[A-Z][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*\b/],
  ['op', /^(?:[=!<>]=|&&|\|\||[-+*/%<>=!])/],
  ['punct', /^[(){}[\],;:]/],
  ['plain', /^[A-Za-z_][A-Za-z0-9_]*/],
  ['plain', /^\s+/]
];

const JSON_TOKENS = [
  ['key', /^"(?:[^"\\]|\\.)*"(?=\s*:)/],
  ['string', /^"(?:[^"\\]|\\.)*"/],
  ['number', /^-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/],
  ['keyword', /^(?:true|false|null)\b/],
  ['punct', /^[{}[\],:]/],
  ['plain', /^\s+/]
];

function tokenize(source, rules) {
  let out = '';
  let rest = source;
  while (rest) {
    let matched = null;
    for (const [kind, pattern] of rules) {
      const hit = pattern.exec(rest);
      if (hit) {
        matched = [kind, hit[0]];
        break;
      }
    }
    // An unrecognized character is emitted as-is rather than dropped, so the
    // highlighted layer always lines up character-for-character with the
    // textarea it sits behind.
    const [kind, text] = matched ?? ['plain', rest[0]];
    out += kind === 'plain' ? escapeHtml(text) : `<span class="t-${kind}">${escapeHtml(text)}</span>`;
    rest = rest.slice(text.length);
  }
  return out;
}

/** GRL source as highlighted HTML. Input is escaped; output is trusted markup. */
export function highlightGrl(source) {
  return tokenize(source, GRL_TOKENS);
}

/** JSON text as highlighted HTML. Highlights invalid JSON too — it is lexical. */
export function highlightJson(source) {
  return tokenize(source, JSON_TOKENS);
}
