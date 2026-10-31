// Ctrl+G: go to a line (CodeMirror's own gotoLine panel is not in the packages we ship, so this is a tiny prompt).
import { EditorView } from '@codemirror/view';
import { promptDialog } from './dom';

export function gotoLine(view: EditorView): boolean {
  const total = view.state.doc.lines;
  void promptDialog('Go to line', `Line number (1–${total})`, '', 'Go').then((v) => {
    const n = Number((v ?? '').trim());
    if (!Number.isInteger(n) || n < 1) return view.focus();
    const line = view.state.doc.line(Math.min(n, total));
    view.dispatch({ selection: { anchor: line.from }, effects: EditorView.scrollIntoView(line.from, { y: 'center' }) });
    view.focus();
  });
  return true;
}
