// The call room's code editor: CodeMirror 6 with the languages Onsite offers.
// Exposed as window.OnsiteEditor.create(parent, {doc, lang, onChange}).
import { EditorView, keymap, lineNumbers, highlightActiveLine, highlightActiveLineGutter, drawSelection,
  dropCursor, rectangularSelection, crosshairCursor, highlightSpecialChars } from "@codemirror/view";
import { EditorState, Compartment, Transaction } from "@codemirror/state";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { indentOnInput, indentUnit, bracketMatching, foldGutter, foldKeymap, syntaxHighlighting,
  HighlightStyle, syntaxTree } from "@codemirror/language";
import { closeBrackets, closeBracketsKeymap, autocompletion, completionKeymap, completeAnyWord,
  completeFromList } from "@codemirror/autocomplete";
import { searchKeymap, highlightSelectionMatches } from "@codemirror/search";
import { linter, lintGutter, lintKeymap } from "@codemirror/lint";
import { tags as t } from "@lezer/highlight";
import { java } from "@codemirror/lang-java";
import { python } from "@codemirror/lang-python";
import { cpp } from "@codemirror/lang-cpp";
import { javascript } from "@codemirror/lang-javascript";
import { go } from "@codemirror/lang-go";

const KEYWORDS = {
  Java: "abstract boolean break byte case catch char class continue default do double else enum extends final finally float for if implements import instanceof int interface long new null package private protected public return short static super switch this throw throws try void volatile while var record true false String Integer Long Character List ArrayList Map HashMap Set HashSet Deque ArrayDeque Queue LinkedList PriorityQueue Arrays Collections Math StringBuilder Comparator Optional",
  Python: "and as assert break class continue def del elif else except False finally for from global if import in is lambda None nonlocal not or pass raise return True try while with yield self len range enumerate zip sorted reversed min max sum abs list dict set tuple str int float bool print collections deque defaultdict Counter heapq heappush heappop bisect functools lru_cache cache Optional List Dict",
  "C++": "auto bool break case catch char class const continue default delete do double else enum explicit false float for friend if inline int long namespace new nullptr operator private protected public return short signed sizeof static struct switch template this throw true try typedef typename unsigned using virtual void while vector string unordered_map unordered_set map set pair queue stack deque priority_queue make_pair push_back emplace_back size begin end sort reverse min max std",
  JavaScript: "async await break case catch class const continue default delete do else export extends false finally for function if import in instanceof let new null return static super switch this throw true try typeof undefined var void while yield Array Map Set Object Math Number String JSON Infinity length push pop shift unshift slice splice sort",
  TypeScript: "async await break case catch class const continue default delete do else export extends false finally for function if implements import in instanceof interface let new null number string boolean private public readonly return static super switch this throw true try type typeof undefined var void while Array Map Set Record Math",
  Go: "break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var append cap len make new nil true false int int64 string bool byte rune float64 error fmt sort strings",
};

function language(lang) {
  switch (lang) {
    case "Java": return java();
    case "Python": return python();
    case "C++": return cpp();
    case "JavaScript": return javascript();
    case "TypeScript": return javascript({ typescript: true });
    case "Go": return go();
    default: return [];
  }
}

/* Keywords and common library names for the language, plus any word already in the file. */
function completions(lang) {
  const words = (KEYWORDS[lang] || "").split(" ").filter(Boolean).map(label => ({ label, type: "keyword" }));
  return EditorState.languageData.of(() => [{ autocomplete: completeFromList(words) }, { autocomplete: completeAnyWord }]);
}

/* Lint: the parser's own syntax errors (missing brackets, stray tokens). Compile errors come from Run. */
const syntaxLint = linter(view => {
  const out = [], doc = view.state.doc;
  syntaxTree(view.state).cursor().iterate(node => {
    if (!node.type.isError || out.length >= 20) return;
    // Errors are often zero-width (a missing brace at the end): mark at least one character.
    const from = node.from >= doc.length ? Math.max(0, doc.length - 1) : node.from;
    const to = Math.max(node.to, Math.min(doc.length, from + 1));
    if (out.some(d => d.from === from)) return;
    out.push({ from, to, severity: "error", message: "Syntax error here." });
  });
  return out;
}, { delay: 900 });

/* Colours come from app.css (--syn-* and the surface tokens), so the editor follows the app theme. */
const theme = EditorView.theme({
  "&": { height: "100%", background: "var(--code-bg)", color: "var(--ink)", fontSize: "13.5px" },
  ".cm-scroller": { fontFamily: "var(--mono)", lineHeight: "1.6", fontVariantLigatures: "none" },
  ".cm-content": { padding: "12px 0", caretColor: "var(--accent-hi)" },
  ".cm-line": { padding: "0 14px 0 10px" },
  "&.cm-focused": { outline: "none" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--accent-hi)", borderLeftWidth: "2px" },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection":
    { background: "var(--syn-select) !important" },
  ".cm-gutters": { background: "var(--code-bg)", color: "var(--syn-gutter)", border: "none", borderRight: "1px solid var(--line)" },
  ".cm-lineNumbers .cm-gutterElement": { padding: "0 8px 0 12px", minWidth: "40px" },
  ".cm-activeLine": { background: "var(--syn-line)" },
  ".cm-activeLineGutter": { background: "var(--syn-line)", color: "var(--ink-2)" },
  ".cm-matchingBracket, &.cm-focused .cm-matchingBracket": { background: "var(--accent-soft)", outline: "1px solid var(--accent)" },
  ".cm-nonmatchingBracket": { color: "var(--bad)" },
  ".cm-selectionMatch": { background: "var(--syn-match)" },
  ".cm-foldPlaceholder": { background: "var(--surface-2)", border: "1px solid var(--line)", color: "var(--muted)" },
  ".cm-tooltip": { background: "var(--surface-2)", border: "1px solid var(--line-2)", borderRadius: "9px", color: "var(--ink)", overflow: "hidden" },
  ".cm-tooltip-autocomplete > ul": { fontFamily: "var(--mono)", fontSize: "12.5px", maxHeight: "14em" },
  ".cm-tooltip-autocomplete > ul > li": { padding: "3px 10px !important" },
  ".cm-tooltip-autocomplete > ul > li[aria-selected]": { background: "var(--accent-soft)", color: "var(--ink)" },
  ".cm-completionDetail": { color: "var(--muted)" },
  ".cm-completionIcon": { opacity: ".6" },
  ".cm-diagnostic-error": { borderLeftColor: "var(--bad)" },
  ".cm-lintRange-error": { backgroundImage: "none", textDecoration: "underline wavy var(--bad)", textUnderlineOffset: "3px" },
  ".cm-panels": { background: "var(--surface)", color: "var(--ink)" },
  ".cm-panels.cm-panels-top": { borderBottom: "1px solid var(--line)" },
  ".cm-search input, .cm-search button": { fontSize: "12.5px" },
  ".cm-textfield": { background: "var(--field)", border: "1px solid var(--line)", borderRadius: "6px", color: "var(--ink)" },
  ".cm-button": { background: "var(--surface-2)", backgroundImage: "none", border: "1px solid var(--line-2)", borderRadius: "6px", color: "var(--ink)" },
  ".cm-searchMatch": { background: "var(--syn-match)", outline: "1px solid var(--line-2)" },
  ".cm-searchMatch.cm-searchMatch-selected": { background: "var(--accent-soft)" },
}, { dark: true });

const highlight = HighlightStyle.define([
  { tag: [t.keyword, t.modifier, t.controlKeyword, t.operatorKeyword, t.definitionKeyword, t.moduleKeyword], color: "var(--syn-kw)" },
  { tag: [t.typeName, t.className, t.namespace, t.standard(t.typeName)], color: "var(--syn-type)" },
  { tag: [t.function(t.variableName), t.function(t.propertyName), t.definition(t.function(t.variableName))], color: "var(--syn-fn)" },
  { tag: [t.string, t.special(t.string), t.character, t.regexp], color: "var(--syn-str)" },
  { tag: [t.number, t.bool, t.null, t.atom], color: "var(--syn-num)" },
  { tag: [t.comment, t.lineComment, t.blockComment, t.docComment], color: "var(--syn-comment)", fontStyle: "italic" },
  { tag: [t.operator, t.punctuation, t.bracket, t.separator], color: "var(--ink-2)" },
  { tag: [t.meta, t.annotation, t.processingInstruction], color: "var(--syn-meta)" },
  { tag: [t.propertyName], color: "var(--ink)" },
  { tag: t.invalid, color: "var(--bad)" },
]);

function create(parent, { doc = "", lang = "Java", onChange } = {}) {
  const langConf = new Compartment(), wordsConf = new Compartment();
  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc,
      extensions: [
        lineNumbers(), highlightActiveLineGutter(), foldGutter(), lintGutter(),
        highlightSpecialChars(), history(), drawSelection(), dropCursor(),
        EditorState.allowMultipleSelections.of(true), rectangularSelection(), crosshairCursor(),
        indentOnInput(), indentUnit.of("    "), EditorState.tabSize.of(4),
        bracketMatching(), closeBrackets(), autocompletion({ activateOnTyping: true, maxRenderedOptions: 40 }),
        highlightActiveLine(), highlightSelectionMatches(),
        syntaxHighlighting(highlight), theme, syntaxLint,
        langConf.of(language(lang)), wordsConf.of(completions(lang)),
        keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...searchKeymap, ...historyKeymap,
          ...foldKeymap, ...completionKeymap, ...lintKeymap, indentWithTab]),
        EditorView.contentAttributes.of({ "aria-label": "Code editor", spellcheck: "false", autocorrect: "off", autocapitalize: "off" }),
        EditorView.updateListener.of(u => { if (u.docChanged && onChange) onChange(); }),
      ],
    }),
  });
  return {
    view,
    get: () => view.state.doc.toString(),
    // Loading a buffer (new round, other language) is not an edit, so Undo can't step back into it.
    set(text) { view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text || "" }, annotations: Transaction.addToHistory.of(false) }); },
    setLang(l) { view.dispatch({ effects: [langConf.reconfigure(language(l)), wordsConf.reconfigure(completions(l))] }); },
    focus: () => view.focus(),
  };
}

window.OnsiteEditor = { create };
