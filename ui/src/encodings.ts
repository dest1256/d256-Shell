import { t } from './i18n';
// Encodings offered by the editor (names are WHATWG labels; the backend knows exactly the same list).
export const ENCODINGS: [string, string][] = [
  ['UTF-8', t('Unicode (UTF-8)')],
  ['UTF-16LE', t('Unicode (UTF-16 LE, as in Windows)')],
  ['UTF-16BE', t('Unicode (UTF-16 BE)')],
  ['windows-1251', t('Cyrillic (Windows-1251)')],
  ['KOI8-R', t('Cyrillic (KOI8-R)')],
  ['KOI8-U', t('Cyrillic (KOI8-U)')],
  ['IBM866', t('Cyrillic (DOS, CP866)')],
  ['ISO-8859-5', t('Cyrillic (ISO 8859-5)')],
  ['x-mac-cyrillic', t('Cyrillic (Mac)')],
  ['windows-1252', t('Western Europe (Windows-1252 / Latin-1)')],
  ['ISO-8859-15', t('Western Europe (Latin-9)')],
  ['windows-1250', t('Central Europe (Windows-1250)')],
  ['ISO-8859-2', t('Central Europe (ISO 8859-2)')],
  ['windows-1253', t('Greek (Windows-1253)')],
  ['windows-1254', t('Turkish (Windows-1254)')],
  ['windows-1255', t('Hebrew (Windows-1255)')],
  ['windows-1256', t('Arabic (Windows-1256)')],
  ['windows-1257', t('Baltic (Windows-1257)')],
  ['windows-1258', t('Vietnamese (Windows-1258)')],
  ['windows-874', t('Thai (Windows-874)')],
  ['Shift_JIS', t('Japanese (Shift_JIS)')],
  ['EUC-JP', t('Japanese (EUC-JP)')],
  ['ISO-2022-JP', t('Japanese (ISO-2022-JP)')],
  ['GBK', t('Chinese Simplified (GBK)')],
  ['gb18030', t('Chinese (GB18030)')],
  ['Big5', t('Chinese Traditional (Big5)')],
  ['EUC-KR', t('Korean (EUC-KR)')],
  ['macintosh', t('Western Europe (Mac Roman)')],
];

export const encodingTitle = (name: string): string => ENCODINGS.find(([n]) => n === name)?.[1] ?? name;
/** Only the Unicode encodings have a byte order mark. */
export const supportsBom = (name: string): boolean => name === 'UTF-8' || name.startsWith('UTF-16');
