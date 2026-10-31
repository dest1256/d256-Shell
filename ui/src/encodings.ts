// Encodings offered by the editor (names are WHATWG labels; the backend knows exactly the same list).
export const ENCODINGS: [string, string][] = [
  ['UTF-8', 'Unicode (UTF-8)'],
  ['UTF-16LE', 'Unicode (UTF-16 LE, as in Windows)'],
  ['UTF-16BE', 'Unicode (UTF-16 BE)'],
  ['windows-1251', 'Cyrillic (Windows-1251)'],
  ['KOI8-R', 'Cyrillic (KOI8-R)'],
  ['KOI8-U', 'Cyrillic (KOI8-U)'],
  ['IBM866', 'Cyrillic (DOS, CP866)'],
  ['ISO-8859-5', 'Cyrillic (ISO 8859-5)'],
  ['x-mac-cyrillic', 'Cyrillic (Mac)'],
  ['windows-1252', 'Western Europe (Windows-1252 / Latin-1)'],
  ['ISO-8859-15', 'Western Europe (Latin-9)'],
  ['windows-1250', 'Central Europe (Windows-1250)'],
  ['ISO-8859-2', 'Central Europe (ISO 8859-2)'],
  ['windows-1253', 'Greek (Windows-1253)'],
  ['windows-1254', 'Turkish (Windows-1254)'],
  ['windows-1255', 'Hebrew (Windows-1255)'],
  ['windows-1256', 'Arabic (Windows-1256)'],
  ['windows-1257', 'Baltic (Windows-1257)'],
  ['windows-1258', 'Vietnamese (Windows-1258)'],
  ['windows-874', 'Thai (Windows-874)'],
  ['Shift_JIS', 'Japanese (Shift_JIS)'],
  ['EUC-JP', 'Japanese (EUC-JP)'],
  ['ISO-2022-JP', 'Japanese (ISO-2022-JP)'],
  ['GBK', 'Chinese Simplified (GBK)'],
  ['gb18030', 'Chinese (GB18030)'],
  ['Big5', 'Chinese Traditional (Big5)'],
  ['EUC-KR', 'Korean (EUC-KR)'],
  ['macintosh', 'Western Europe (Mac Roman)'],
];

export const encodingTitle = (name: string): string => ENCODINGS.find(([n]) => n === name)?.[1] ?? name;
/** Only the Unicode encodings have a byte order mark. */
export const supportsBom = (name: string): boolean => name === 'UTF-8' || name.startsWith('UTF-16');
