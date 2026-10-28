// Encodings offered by the editor (names are WHATWG labels; the backend knows exactly the same list).
export const ENCODINGS: [string, string][] = [
  ['UTF-8', 'Юникод (UTF-8)'],
  ['UTF-16LE', 'Юникод (UTF-16 LE, как в Windows)'],
  ['UTF-16BE', 'Юникод (UTF-16 BE)'],
  ['windows-1251', 'Кириллица (Windows-1251)'],
  ['KOI8-R', 'Кириллица (KOI8-R)'],
  ['KOI8-U', 'Кириллица (KOI8-U)'],
  ['IBM866', 'Кириллица (DOS, CP866)'],
  ['ISO-8859-5', 'Кириллица (ISO 8859-5)'],
  ['x-mac-cyrillic', 'Кириллица (Mac)'],
  ['windows-1252', 'Западная Европа (Windows-1252 / Latin-1)'],
  ['ISO-8859-15', 'Западная Европа (Latin-9)'],
  ['windows-1250', 'Центральная Европа (Windows-1250)'],
  ['ISO-8859-2', 'Центральная Европа (ISO 8859-2)'],
  ['windows-1253', 'Греческий (Windows-1253)'],
  ['windows-1254', 'Турецкий (Windows-1254)'],
  ['windows-1255', 'Иврит (Windows-1255)'],
  ['windows-1256', 'Арабский (Windows-1256)'],
  ['windows-1257', 'Балтийский (Windows-1257)'],
  ['windows-1258', 'Вьетнамский (Windows-1258)'],
  ['windows-874', 'Тайский (Windows-874)'],
  ['Shift_JIS', 'Японский (Shift_JIS)'],
  ['EUC-JP', 'Японский (EUC-JP)'],
  ['ISO-2022-JP', 'Японский (ISO-2022-JP)'],
  ['GBK', 'Китайский упрощённый (GBK)'],
  ['gb18030', 'Китайский (GB18030)'],
  ['Big5', 'Китайский традиционный (Big5)'],
  ['EUC-KR', 'Корейский (EUC-KR)'],
  ['macintosh', 'Западная Европа (Mac Roman)'],
];

export const encodingTitle = (name: string): string => ENCODINGS.find(([n]) => n === name)?.[1] ?? name;
/** Only the Unicode encodings have a byte order mark. */
export const supportsBom = (name: string): boolean => name === 'UTF-8' || name.startsWith('UTF-16');
