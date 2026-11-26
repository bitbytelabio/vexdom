/** Base64 for bytes and UTF-8 text, in chunks so a large body does not overflow the call stack. */
export function bytesToBase64(bytes: Uint8Array): string {
  let binary = '';
  const CHUNK = 0x8000;
  for (let at = 0; at < bytes.length; at += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(at, at + CHUNK));
  }
  return btoa(binary);
}

export const textToBase64 = (text: string): string => bytesToBase64(new TextEncoder().encode(text));

export const base64ToBytes = (base64: string): Uint8Array => Uint8Array.from(atob(base64), (char) => char.charCodeAt(0));
