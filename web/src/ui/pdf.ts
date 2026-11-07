import { GlobalWorkerOptions, getDocument, type PDFDocumentProxy } from 'pdfjs-dist';
import worker from 'pdfjs-dist/build/pdf.worker.min.mjs?url';

GlobalWorkerOptions.workerSrc = worker;

export type Pdf = PDFDocumentProxy;

const COVER_WIDTH = 240;

/**
 * Opens a PDF, with `close` to free its worker. A URL is read in ranges as pages are drawn; pdf.js
 * takes `bytes` over, so pass a copy you still need.
 */
export async function loadPdf(source: Uint8Array | string): Promise<{ pdf: Pdf; close: () => Promise<void> }> {
  const task = getDocument(typeof source === 'string' ? { url: source, disableAutoFetch: true, disableStream: true } : { data: source });
  try {
    return { pdf: await task.promise, close: () => task.destroy() };
  } catch (error) {
    void task.destroy();
    throw error;
  }
}

/** A fresh canvas with page `number` drawn as large as fits `box` (CSS pixels), sharp on this screen. */
export function drawPage(pdf: Pdf, number: number, box: { width: number; height: number }) {
  let cancel = () => {};
  const done = (async () => {
    const page = await pdf.getPage(number);
    const base = page.getViewport({ scale: 1 });
    const scale = Math.min(box.width / base.width, box.height / base.height);
    const viewport = page.getViewport({ scale: scale * (window.devicePixelRatio || 1) });
    const canvas = document.createElement('canvas');
    canvas.width = Math.floor(viewport.width);
    canvas.height = Math.floor(viewport.height);
    canvas.style.width = `${Math.floor(base.width * scale)}px`;
    canvas.style.height = `${Math.floor(base.height * scale)}px`;
    const task = page.render({ canvas, viewport, background: '#ffffff' });
    cancel = () => task.cancel();
    await task.promise;
    return canvas;
  })();
  return { done, cancel: () => cancel() };
}

/** The page's text, lines as pdf.js found them. */
export async function pageText(pdf: Pdf, number: number): Promise<string> {
  const content = await (await pdf.getPage(number)).getTextContent();
  return content.items.map((item) => ('str' in item ? item.str + (item.hasEOL ? '\n' : '') : '')).join('');
}

/** The text of every page, in order; a page without text is empty. */
export async function allText(pdf: Pdf): Promise<string[]> {
  const texts: string[] = [];
  for (let number = 1; number <= pdf.numPages; number++) texts.push(await pageText(pdf, number));
  return texts;
}

/** The first page as a base64 JPEG. */
export async function coverOf(pdf: Pdf): Promise<string> {
  const page = await pdf.getPage(1);
  const base = page.getViewport({ scale: 1 });
  const viewport = page.getViewport({ scale: COVER_WIDTH / base.width });
  const canvas = document.createElement('canvas');
  canvas.width = Math.round(viewport.width);
  canvas.height = Math.round(viewport.height);
  await page.render({ canvas, viewport, background: '#ffffff' }).promise;
  return canvas.toDataURL('image/jpeg', 0.85).split(',')[1];
}

/** `file` in base64, without holding it twice as a JS string of bytes. */
export function base64Of(file: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).split(',')[1] ?? '');
    reader.onerror = () => reject(reader.error ?? new Error('파일을 읽지 못했어요'));
    reader.readAsDataURL(file);
  });
}
