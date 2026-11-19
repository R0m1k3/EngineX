// Icône d'un fichier selon son extension (arbre et barre du fichier ouvert).
import File from '@lucide/svelte/icons/file';
import FileText from '@lucide/svelte/icons/file-text';
import FileCode from '@lucide/svelte/icons/file-code';
import FileImage from '@lucide/svelte/icons/file-image';
import FileBraces from '@lucide/svelte/icons/file-braces';
import FilePlay from '@lucide/svelte/icons/file-play';
import FileMusic from '@lucide/svelte/icons/file-music';
import FileArchive from '@lucide/svelte/icons/file-archive';
import FileTerminal from '@lucide/svelte/icons/file-terminal';
import FileCog from '@lucide/svelte/icons/file-cog';
import FileLock from '@lucide/svelte/icons/file-lock';
import FileSpreadsheet from '@lucide/svelte/icons/file-spreadsheet';
import FileBox from '@lucide/svelte/icons/file-box';
import type { IconType } from '../../lib/ui';

const BY_EXT: Record<string, IconType> = {};
const put = (icon: IconType, exts: string) => exts.split(' ').forEach((e) => (BY_EXT[e] = icon));
put(FileText, 'md markdown txt rst log');
put(FileCode, 'ts tsx js jsx mjs cjs svelte vue rs py go rb java kt c h cpp hpp cs php html htm css scss sass less lua swift sql');
put(FileBraces, 'json jsonc json5 yaml yml toml xml');
put(FileImage, 'png jpg jpeg gif webp avif svg ico bmp');
put(FilePlay, 'mp4 webm mov mkv avi');
put(FileMusic, 'mp3 ogg wav flac m4a');
put(FileArchive, 'zip gz tgz tar xz bz2 7z rar');
put(FileTerminal, 'sh bash zsh fish ps1 bat');
put(FileCog, 'ini conf cfg env');
put(FileLock, 'lock key pem');
put(FileSpreadsheet, 'csv tsv xlsx ods');
put(FileBox, 'gguf bin exe so dylib wasm pdf');

const BY_NAME: Record<string, IconType> = {
  Dockerfile: FileCode,
  Makefile: FileTerminal,
  '.gitignore': FileCog,
  '.env': FileCog,
};

export function fileIcon(name: string): IconType {
  if (BY_NAME[name]) return BY_NAME[name];
  const dot = name.lastIndexOf('.');
  return (dot >= 0 && BY_EXT[name.slice(dot + 1).toLowerCase()]) || File;
}
