// Éditeur CodeMirror 6, chargé à la demande (page Fichiers).
import { EditorView, basicSetup } from 'codemirror';
import { EditorState, Compartment } from '@codemirror/state';
import { keymap } from '@codemirror/view';
import { indentWithTab } from '@codemirror/commands';
import { oneDark } from '@codemirror/theme-one-dark';
import type { Extension } from '@codemirror/state';

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
  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc: opts.text,
      extensions: [
        basicSetup,
        oneDark,
        keymap.of([indentWithTab, { key: 'Mod-s', preventDefault: true, run: () => (opts.onSave(), true) }]),
        EditorState.readOnly.of(opts.readonly),
        EditorView.updateListener.of((u) => u.docChanged && opts.onChange()),
        EditorView.theme({ '&': { height: '100%', fontSize: '13px' }, '.cm-scroller': { fontFamily: 'ui-monospace, monospace' } }),
        lang.of([]),
      ],
    }),
  });
  language(opts.path).then((l) => view.dispatch({ effects: lang.reconfigure(l) }));
  return {
    text: () => view.state.doc.toString(),
    set: (text) => view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } }),
    destroy: () => view.destroy(),
  };
}
