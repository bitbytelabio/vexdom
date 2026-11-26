import { z } from 'zod';
import { ActionError, defineAction } from '../core';
import { UNDESCRIBED, describeElement, resolveTarget, targetSchema } from './dom';

export const attachFile = defineAction({
  name: 'page.attachFile',
  description:
    'Attach a file to a file input on the page: one the user stored in Vexdom (fileId, from page.listFiles), one you captured off another page (downloadId, from page.captureDownload), or local files by path (paths). The second closes the loop — download here, upload there — without the bytes ever passing through you.',
  input: z.object({
    fileId: z.string().optional().describe('Id of a stored file, taken from page.listFiles.'),
    downloadId: z
      .string()
      .optional()
      .describe('Id of a captured download, taken from page.captureDownload or page.listDownloads.'),
    paths: z
      .array(z.string())
      .optional()
      .describe('Absolute paths of local files on the computer running the Vexdom Bridge.'),
    target: targetSchema.describe('The file input (<input type="file">) to attach the file to.'),
    name: z.string().optional().describe('Internal: original filename. Vexdom fills this in.'),
    mime: z.string().optional().describe('Internal: file MIME type. Vexdom fills this in.'),
    content: z.string().optional().describe('Internal: base64 file bytes. Vexdom fills this in.'),
    files: z
      .array(z.object({ name: z.string(), mime: z.string(), content: z.string() }))
      .optional()
      .meta({ [UNDESCRIBED]: true }),
  }),
  execute({ target, name, mime, content, files }) {
    const given = files ?? (content ? [{ name: name || 'file', mime: mime || 'application/octet-stream', content }] : []);
    if (!given.length) {
      throw new ActionError('No file bytes were supplied — call with a valid fileId, downloadId or paths.', 'INVALID_INPUT');
    }
    const el = resolveTarget(target, { includeHidden: true });
    if (!(el instanceof HTMLInputElement) || el.type !== 'file') {
      throw new ActionError(`<${el.tagName.toLowerCase()}> is not a file input`, 'INVALID_TARGET');
    }
    if (given.length > 1 && !el.multiple) {
      throw new ActionError(`This file input takes one file; ${given.length} were given.`, 'NOT_MULTIPLE');
    }

    const transfer = new DataTransfer();
    const attached = given.map((part) => {
      const binary = atob(part.content);
      const bytes = new Uint8Array(binary.length);
      for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
      const file = new File([bytes], part.name || 'file', { type: part.mime || 'application/octet-stream' });
      transfer.items.add(file);
      return { name: file.name, size: bytes.length, mime: file.type };
    });
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'files')!.set!.call(el, transfer.files);
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));

    if (attached.length === 1) return { attached: true, ...attached[0], element: describeElement(el) };
    return { attached: true, files: attached, element: describeElement(el) };
  },
});
