// Duplicating a terminal: a second shell on the same server that starts where the first one is: the same folder,
// the same virtualenv, the same foreground program (btop, tail -f ...). What is needed is read from the server's
// /proc over SFTP; the commands are then typed into the NEW shell, after showing them to the user.
import { api, ApiError, TermContext } from './api';
import { h, modal, toast } from './dom';
import { getSettings, updateSettings } from './settings';
import { TerminalView } from './terminal';

/** Quote one word for a POSIX shell (ash, bash, zsh). */
export function shq(s: string): string {
  return /^[A-Za-z0-9_@%+=:,./-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\''`)}'`;
}

/** Full-screen programs: their screen is a picture of one moment, useless (and ugly) as scrollback of a copy. */
const FULLSCREEN = new Set(['btop', 'bpytop', 'htop', 'top', 'atop', 'glances', 'nmon', 'iotop', 'iftop', 'nvtop', 'ncdu', 'mc', 'vim', 'vi', 'nvim', 'nano', 'less', 'more', 'man', 'tmux', 'screen', 'watch', 'emacs']);

const SHELLS = new Set(['sh', 'ash', 'bash', 'dash', 'zsh', 'fish', 'ksh', 'tcsh', 'csh', 'busybox']);

export interface ClonePlan {
  /** Show the old output above the new shell (off by default when a full-screen program runs). */
  prefill: boolean;
  lines: string[];
  /** Commands of an environment (venv) that stay "active" in the copy, for duplicating the copy again. */
  env: string[];
  summary: string[];
}

/** Turn what was found on the server (and what the user typed to activate a venv) into shell lines. */
export function buildPlan(ctx: TermContext, typedEnv: string[]): ClonePlan {
  const lines: string[] = [];
  const summary: string[] = [];
  let env: string[] = [];
  const nested = ctx.nested;
  const launcher = nested ? nested.launcher.map(shq).join(' ') : '';
  if (launcher) summary.push(`login: ${nested!.launcher.join(' ')}`);
  if (ctx.cwd) {
    lines.push(`cd ${shq(ctx.cwd)}`);
    summary.push(`folder ${ctx.cwd}`);
  }
  const e = ctx.env ?? {};
  if (e.VIRTUAL_ENV) {
    env = [`. ${shq(e.VIRTUAL_ENV + '/bin/activate')}`];
    summary.push(`venv ${e.VIRTUAL_ENV}`);
  } else if (e.CONDA_DEFAULT_ENV) {
    env = [`conda activate ${shq(e.CONDA_DEFAULT_ENV)}`];
    summary.push(`conda ${e.CONDA_DEFAULT_ENV}`);
  } else if (typedEnv.length) {
    env = typedEnv;
    summary.push('the environment you activated in this terminal');
  }
  lines.push(...env);
  const fg = ctx.fg;
  const comm = (ctx.fg_comm ?? '').trim();
  if (fg?.length && !fg[0].startsWith('-') && !SHELLS.has(comm) && !SHELLS.has(fg[0].split('/').pop() ?? '')) {
    lines.push(fg.map(shq).join(' '));
    summary.push(`running: ${fg.join(' ')}`);
  }
  const prog = (fg?.[0] ?? '').split('/').pop() ?? '';
  if (launcher) {
    if (ctx.root) lines.unshift(launcher);
    else {
      // su / sudo / ssh as an ordinary user ask for a password: nothing may be typed after the launcher
      summary.push('next (folder, environment, program) are typed manually after the password');
      lines.splice(0, lines.length, launcher);
    }
  }
  return { prefill: !FULLSCREEN.has(prog) && !FULLSCREEN.has(comm), lines, env, summary };
}

/** Ask the user what to type into the copy. Resolves to the lines, or null for "cancel". */
function confirmPlan(ctx: TermContext, plan: ClonePlan): Promise<{ lines: string[]; prefill: boolean } | null> {
  return modal<{ lines: string[]; prefill: boolean } | null>((close) => {
    const ta = h('textarea', { rows: '5', spellcheck: 'false', 'data-testid': 'clone-lines' }) as HTMLTextAreaElement;
    ta.value = plan.lines.join('\n');
    const remember = h('input', { type: 'checkbox', 'data-testid': 'clone-remember' }) as HTMLInputElement;
    const showOld = h('input', { type: 'checkbox', 'data-testid': 'clone-prefill' }) as HTMLInputElement;
    showOld.checked = plan.prefill;
    const done = async (lines: string[] | null) => {
      // saved before the dialog closes: the next Ctrl+Shift+D must already see the new mode
      if (remember.checked && lines) await updateSettings({ clone_mode: lines.length ? 'auto' : 'view' }).catch(() => undefined);
      close(lines ? { lines, prefill: showOld.checked } : null);
    };
    const found = ctx.found
      ? plan.summary.length
        ? `Found on the server: ${plan.summary.join(' · ')}.`
        : 'The shell was found: it is at the prompt in the home folder, nothing to repeat.'
      : `Could not determine the state of the terminal: ${ctx.reason ?? ''}`;
    return h(
      'form',
      { role: 'dialog', 'data-testid': 'clone-dialog', class: 'wide', onsubmit: (e) => (e.preventDefault(), void done(ta.value.split('\n').map((s) => s.trim()).filter(Boolean))) },
      h('h3', {}, 'Duplicate terminal'),
      h('p', {}, found),
      h('p', { class: 'muted' }, 'These commands will be typed into the copy, one per line. You can change or remove them. The state of the running program (its internal data) is not copied: it will start again.'),
      ta,
      h('label', { class: 'check' }, showOld, ' Show the earlier terminal output on top'),
      h('label', { class: 'check' }, remember, ' Do not ask again (can be restored in Settings)'),
      h(
        'div',
        { class: 'row end wrap' },
        h('button', { type: 'button', onclick: () => close(null), 'data-testid': 'clone-cancel' }, 'Cancel'),
        h('button', { type: 'button', onclick: () => void done([]), 'data-testid': 'clone-view' }, 'Output only, no commands'),
        h('button', { type: 'submit', class: 'primary', 'data-testid': 'clone-ok' }, 'Duplicate'),
      ),
    );
  });
}

export interface CloneResult {
  /** Screen and scrollback of the source, shown first in the copy. */
  /** Screen and scrollback of the source to show first, or empty. */
  prefill: string;
  startup: string[];
  env: string[];
}

/** Everything needed to open a copy of `src`, or null when the user cancelled. */
export async function prepareClone(src: TerminalView): Promise<CloneResult | null> {
  const mode = getSettings().clone_mode;
  const prefill = src.snapshot();
  if (mode === 'view' || !src.id) return { prefill, startup: [], env: src.envCommands };
  let ctx: TermContext;
  try {
    // a busy server (hundreds of processes) takes a moment: say so, and never wait forever
    toast('Finding out what is going on in the terminal…');
    ctx = await Promise.race([
      api.termContext(src.id),
      new Promise<never>((_, rej) => setTimeout(() => rej(new Error('the server takes too long to respond')), 25_000)),
    ]);
  } catch (e) {
    ctx = { found: false, reason: e instanceof ApiError ? e.message : String(e) };
    if (mode === 'auto') toast(`Copy without commands: ${ctx.reason}`);
  }
  const plan = buildPlan(ctx, src.envCommands);
  if (mode === 'auto' || (!plan.lines.length && ctx.found)) return { prefill: plan.prefill ? prefill : '', startup: plan.lines, env: plan.env };
  const r = await confirmPlan(ctx, plan);
  if (r === null) return null;
  return { prefill: r.prefill ? prefill : '', startup: r.lines, env: plan.env };
}
