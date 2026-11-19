// Éditeur CodeMirror 6, chargé à la demande (page Fichiers).
import { EditorView, basicSetup } from 'codemirror';
import { EditorState, Compartment } from '@codemirror/state';
import { keymap } from '@codemirror/view';
import { indentWithTab } from '@codemirror/commands';
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { tags as t } from '@lezer/highlight';
import type { Extension } from '@codemirror/state';
import { isDark, onThemeChange } from './ui/theme.svelte';

// Thème EngineX : couleurs prises des jetons CSS (app.css), donc clair et sombre
// suivent la bascule sans reconfigurer ; seul le drapeau « sombre » de CodeMirror
// (curseur, sélection par défaut) passe par un Compartment.
const chrome = EditorView.theme({
  '&': { height: '100%', fontSize: 'var(--fs-code)', color: 'var(--text)', backgroundColor: 'var(--bg)' },
  '.cm-scroller': { fontFamily: 'var(--mono)', lineHeight: '1.6' },
  '.cm-content': { caretColor: 'var(--accent)' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--accent)' },
  '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection':
    { backgroundColor: 'var(--accent-soft)' },
  '.cm-gutters': { backgroundColor: 'var(--panel)', color: 'var(--faint)', borderRight: '1px solid var(--line)' },
  '.cm-activeLine': { backgroundColor: 'color-mix(in srgb, var(--raised) 45%, transparent)' },
  '.cm-activeLineGutter': { backgroundColor: 'var(--raised)', color: 'var(--text)' },
  '.cm-selectionMatch': { backgroundColor: 'color-mix(in srgb, var(--accent-soft) 60%, transparent)' },
  '&.cm-focused .cm-matchingBracket': { backgroundColor: 'var(--accent-soft)', outline: '1px solid var(--accent-2)' },
  '.cm-searchMatch': { backgroundColor: 'var(--warn-soft)', outline: '1px solid var(--warn)' },
  '.cm-searchMatch.cm-searchMatch-selected': { backgroundColor: 'var(--accent-soft)' },
  '.cm-foldPlaceholder': { backgroundColor: 'var(--raised)', border: '1px solid var(--line-strong)', color: 'var(--muted)' },
  '.cm-tooltip': { backgroundColor: 'var(--panel)', border: '1px solid var(--line-strong)', color: 'var(--text)' },
  '.cm-tooltip-autocomplete > ul > li[aria-selected]': { backgroundColor: 'var(--accent-soft)', color: 'var(--text)' },
  '.cm-panels': { backgroundColor: 'var(--panel)', color: 'var(--text)' },
  '.cm-panels.cm-panels-top': { borderBottom: '1px solid var(--line)' },
  '.cm-panels.cm-panels-bottom': { borderTop: '1px solid var(--line)' },
  '.cm-textfield': { backgroundColor: 'var(--bg)', border: '1px solid var(--line-strong)', color: 'var(--text)' },
  '.cm-button': { backgroundImage: 'none', backgroundColor: 'var(--raised)', border: '1px solid var(--line-strong)', color: 'var(--text)' },
});

const highlight = HighlightStyle.define([
  { tag: [t.keyword, t.operatorKeyword, t.modifier, t.controlKeyword], color: 'var(--code-keyword)' },
  { tag: [t.string, t.special(t.string), t.regexp, t.character], color: 'var(--code-string)' },
  { tag: [t.number, t.bool, t.null, t.atom, t.unit], color: 'var(--code-number)' },
  { tag: [t.comment, t.lineComment, t.blockComment, t.docComment], color: 'var(--code-comment)' },
  { tag: [t.function(t.variableName), t.function(t.propertyName), t.macroName], color: 'var(--code-fn)' },
  { tag: [t.typeName, t.className, t.namespace, t.definition(t.typeName)], color: 'var(--code-type)' },
  { tag: [t.tagName, t.heading], color: 'var(--code-tag)', fontWeight: '600' },
  { tag: [t.attributeName, t.propertyName, t.labelName], color: 'var(--code-prop)' },
  { tag: [t.meta, t.processingInstruction, t.punctuation, t.separator], color: 'var(--muted)' },
  { tag: t.link, color: 'var(--accent)', textDecoration: 'underline' },
  { tag: t.emphasis, fontStyle: 'italic' },
  { tag: t.strong, fontWeight: '600' },
  { tag: t.strikethrough, textDecoration: 'line-through' },
  { tag: t.invalid, color: 'var(--danger)' },
]);

async function language(path: string): Promise<Extension> {
  const ext = path.split('.').pop()?.toLowerCase() ?? '';
  switch (ext) {
    case 'js': case 'mjs': case 'cjs': case 'jsx':
      return (await import('@codemirror/lang-javascript')).javascript({ jsx: true });
    case 'ts': case 'tsx': case 'mts':
      return (await import('@codemirror/lang-javascript')).javascript({ typescript: true, jsx: ext === 'tsx' });
    case 'svelte': case 'html': case 'htm': case 'vue':
      return (await import('@codemirror/lang-html')).html();
    case 'css': case 'scss':
      return (await import('@codemirror/lang-css')).css();
    case 'json': case 'jsonc':
      return (await import('@codemirror/lang-json')).json();
    case 'md': case 'markdown':
      return (await import('@codemirror/lang-markdown')).markdown();
    case 'py':
      return (await import('@codemirror/lang-python')).python();
    case 'rs':
      return (await import('@codemirror/lang-rust')).rust();
    case 'yml': case 'yaml':
      return (await import('@codemirror/lang-yaml')).yaml();
    default:
      return [];
  }
}

export interface Editor {
  text(): string;
  set(text: string): void;
  destroy(): void;
}

export async function createEditor(
  parent: HTMLElement,
  opts: { path: string; text: string; readonly: boolean; onChange: () => void; onSave: () => void },
): Promise<Editor> {
  const lang = new Compartment();
  const scheme = new Compartment();
  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc: opts.text,
      extensions: [
        basicSetup,
        chrome,
        syntaxHighlighting(highlight),
        scheme.of(EditorView.darkTheme.of(isDark())),
        keymap.of([indentWithTab, { key: 'Mod-s', preventDefault: true, run: () => (opts.onSave(), true) }]),
        EditorState.readOnly.of(opts.readonly),
        EditorView.updateListener.of((u) => u.docChanged && opts.onChange()),
        lang.of([]),
      ],
    }),
  });
  language(opts.path).then((l) => view.dispatch({ effects: lang.reconfigure(l) }));
  const unwatch = onThemeChange((dark) => view.dispatch({ effects: scheme.reconfigure(EditorView.darkTheme.of(dark)) }));
  return {
    text: () => view.state.doc.toString(),
    set: (text) => view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } }),
    destroy: () => {
      unwatch();
      view.destroy();
    },
  };
}
