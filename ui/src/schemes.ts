// Colour schemes (values as shipped with Windows Terminal).
import type { ITheme } from '@xterm/xterm';

export interface Scheme {
  id: string;
  name: string;
  theme: ITheme;
}

const mk = (
  id: string,
  name: string,
  bg: string,
  fg: string,
  c: [string, string, string, string, string, string, string, string],
  b: [string, string, string, string, string, string, string, string],
  cursor = fg,
  selection = 'rgba(255,255,255,0.25)',
): Scheme => ({
  id,
  name,
  theme: {
    background: bg,
    foreground: fg,
    cursor,
    cursorAccent: bg,
    selectionBackground: selection,
    black: c[0],
    red: c[1],
    green: c[2],
    yellow: c[3],
    blue: c[4],
    magenta: c[5],
    cyan: c[6],
    white: c[7],
    brightBlack: b[0],
    brightRed: b[1],
    brightGreen: b[2],
    brightYellow: b[3],
    brightBlue: b[4],
    brightMagenta: b[5],
    brightCyan: b[6],
    brightWhite: b[7],
  },
});

export const SCHEMES: Scheme[] = [
  mk('campbell', 'Campbell', '#0C0C0C', '#CCCCCC', ['#0C0C0C', '#C50F1F', '#13A10E', '#C19C00', '#0037DA', '#881798', '#3A96DD', '#CCCCCC'], ['#767676', '#E74856', '#16C60C', '#F9F1A5', '#3B78FF', '#B4009E', '#61D6D6', '#F2F2F2'], '#FFFFFF'),
  mk('onehalfdark', 'One Half Dark', '#282C34', '#DCDFE4', ['#282C34', '#E06C75', '#98C379', '#E5C07B', '#61AFEF', '#C678DD', '#56B6C2', '#DCDFE4'], ['#5A6374', '#E06C75', '#98C379', '#E5C07B', '#61AFEF', '#C678DD', '#56B6C2', '#DCDFE4'], '#A3B3CC'),
  mk('onehalflight', 'One Half Light', '#FAFAFA', '#383A42', ['#383A42', '#E45649', '#50A14F', '#C18301', '#0184BC', '#A626A4', '#0997B3', '#FAFAFA'], ['#4F525D', '#DF6C75', '#98C379', '#E4C07A', '#61AFEF', '#C577DD', '#56B5C1', '#FFFFFF'], '#4F525D', 'rgba(0,0,0,0.2)'),
  mk('tangodark', 'Tango Dark', '#000000', '#D3D7CF', ['#000000', '#CC0000', '#4E9A06', '#C4A000', '#3465A4', '#75507B', '#06989A', '#D3D7CF'], ['#555753', '#EF2929', '#8AE234', '#FCE94F', '#729FCF', '#AD7FA8', '#34E2E2', '#EEEEEC'], '#FFFFFF'),
  mk('solarizeddark', 'Solarized Dark', '#002B36', '#839496', ['#002B36', '#DC322F', '#859900', '#B58900', '#268BD2', '#D33682', '#2AA198', '#EEE8D5'], ['#073642', '#CB4B16', '#586E75', '#657B83', '#839496', '#6C71C4', '#93A1A1', '#FDF6E3'], '#839496'),
];

export function schemeById(id: string): Scheme {
  return SCHEMES.find((s) => s.id === id) ?? SCHEMES[0];
}
