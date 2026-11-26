/**
 * The extension's half of the handshake (`docs/research/handshake-crypto.md`). The proofs come from
 * `vexdom-pure` through ext-core, the same code the Bridge's gate runs; nonces and the
 * constant-time compare stay here.
 */
import {
  handshakeClientProof,
  handshakeOpenSessionKey,
  handshakePairingSecret,
  handshakeSealSessionKey,
  handshakeServerProof,
  loadCore,
} from '@/lib/bridge/pure';

const NONCE_BYTES = 16;

export interface Transcript {
  protocolVersion: number;
  extensionVersion: string;
  manifestHash: string;
  clientNonce: string;
  serverNonce: string;
}

export function newNonce(): string {
  return encode(crypto.getRandomValues(new Uint8Array(NONCE_BYTES)));
}

export function isNonce(value: unknown): value is string {
  return typeof value === 'string' && value.length >= 16;
}

/**
 * A pairing code carries ~39 bits, so a squatter who recorded one handshake could grind it on a GPU
 * in seconds if the proof were a plain HMAC. PBKDF2 over the two nonces makes that grind hopeless
 * and leaves nothing to precompute.
 */
export async function pairingSecret(code: string, transcript: Transcript): Promise<string> {
  await loadCore();
  return handshakePairingSecret(code, transcript);
}

export async function clientProof(secret: string, transcript: Transcript): Promise<string> {
  await loadCore();
  return keyed(handshakeClientProof(secret, transcript));
}

export interface Welcome {
  daemonVersion: string;
  manifestHash: string;
  manifestInSync: boolean;
  sealedSessionKey?: string;
}

/**
 * Domain-separated from the client proof so an impostor cannot reflect the extension's own proof
 * back at it. This is what stops a process squatting on a daemon port from being believed, and it
 * covers the whole welcome so none of it can be altered on the way.
 */
export async function serverProof(secret: string, transcript: Transcript, welcome: Welcome): Promise<string> {
  await loadCore();
  return keyed(
    handshakeServerProof(secret, transcript, {
      daemonVersion: welcome.daemonVersion,
      manifestHash: welcome.manifestHash,
      manifestInSync: welcome.manifestInSync,
      sealedSessionKey: welcome.sealedSessionKey ?? null,
    }),
  );
}

export async function sealSessionKey(secret: string, transcript: Transcript, sessionKey: string): Promise<string> {
  await loadCore();
  return keyed(handshakeSealSessionKey(secret, transcript, sessionKey));
}

export async function openSessionKey(
  secret: string,
  transcript: Transcript,
  sealed: string,
): Promise<string | null> {
  await loadCore();
  return handshakeOpenSessionKey(secret, transcript, sealed);
}

export function sameProof(offered: unknown, expected: string): boolean {
  if (typeof offered !== 'string' || offered.length !== expected.length) return false;
  let diff = 0;
  for (let index = 0; index < expected.length; index++) {
    diff |= offered.charCodeAt(index) ^ expected.charCodeAt(index);
  }
  return diff === 0;
}

/** An HMAC needs a key: WebCrypto refused an empty one with this message, and so does this. */
function keyed(proof: string | null): string {
  if (proof === null) throw new Error('Zero-length key is not supported');
  return proof;
}

function encode(bytes: Uint8Array): string {
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}
