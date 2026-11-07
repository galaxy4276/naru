import { open } from '@tauri-apps/plugin-dialog';
import { readFile } from '@tauri-apps/plugin-fs';

export type Picked = {
  name: string;
  /** Where the file is on this Mac; a browser screen does not know. */
  path: string | null;
  blob: () => Promise<Blob>;
};

export type Kind = { label: string; extensions: string[]; accept: string };

export const PDF: Kind = { label: 'PDF', extensions: ['pdf'], accept: 'application/pdf,.pdf' };
export const IMAGE: Kind = { label: '이미지', extensions: ['png', 'jpg', 'jpeg', 'gif', 'webp', 'heic'], accept: 'image/*' };

const inTauri = '__TAURI_INTERNALS__' in window;

/**
 * Files the user picks. In the Tauri window this is the dialog plugin's sheet: WKWebView's own
 * `<input type="file">` panel runs modally on the main thread and freezes the whole app until it
 * closes.
 */
export async function pickFiles(kind: Kind, multiple: boolean): Promise<Picked[]> {
  if (!inTauri) return (await pickInBrowser(kind.accept, multiple)).map((file) => ({ name: file.name, path: null, blob: async () => file }));
  const chosen = await open({ multiple, filters: [{ name: kind.label, extensions: kind.extensions }] });
  const paths = chosen === null ? [] : Array.isArray(chosen) ? chosen : [chosen];
  return paths.map((path) => ({
    name: path.split('/').pop() ?? path,
    path,
    blob: async () => new Blob([await readFile(path)]),
  }));
}

function pickInBrowser(accept: string, multiple: boolean): Promise<File[]> {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = accept;
    input.multiple = multiple;
    input.onchange = () => resolve(Array.from(input.files ?? []));
    input.oncancel = () => resolve([]);
    input.click();
  });
}
