/**
 * Local Vault Secrets Adapter
 *
 * ISecretsPort implementation backed by an AES-256-GCM encrypted JSON file.
 * Uses PBKDF2-SHA512 for key derivation, fresh IV on every write, and
 * atomic file operations (write-to-tmp then rename).
 *
 * Dependencies: node:crypto, node:fs, core/ports, core/domain only.
 */

import { createRequire } from 'node:module';
// Use createRequire to avoid Bun's ESM named-export race under parallel test load
const _require = createRequire(import.meta.url);
const { createCipheriv, createDecipheriv, pbkdf2Sync, randomBytes } =
  _require('node:crypto') as typeof import('node:crypto');
const { readFileSync, renameSync, writeFileSync } =
  _require('node:fs') as typeof import('node:fs');

import type { SecretMetadata, SecretResult } from '../../core/ports/secrets.js';
import type { ISecretsPort } from '../../core/ports/secrets.js';

/* ------------------------------------------------------------------ */
/*  Internal types                                                     */
/* ------------------------------------------------------------------ */

interface VaultEnvelope {
  salt: string;   // hex
  iv: string;     // hex
  tag: string;    // hex
  data: string;   // hex (ciphertext)
  kdf: 'pbkdf2';
  kdfIterations: number;
}

interface VaultEntry {
  value: string;
  createdAt: string;
  updatedAt: string;
  version: number;
}

interface VaultPayload {
  version: 1;
  secrets: Record<string, VaultEntry>;
}

/* ------------------------------------------------------------------ */
/*  Constants                                                          */
/* ------------------------------------------------------------------ */

const DEFAULT_KDF_ITERATIONS = 600_000;
const SALT_BYTES = 32;
const IV_BYTES = 16;
const KEY_BYTES = 32; // AES-256
const AUTH_TAG_LENGTH = 16; // 128-bit GCM tag
const ALGORITHM = 'aes-256-gcm' as const;

/* ------------------------------------------------------------------ */
/*  Adapter                                                            */
/* ------------------------------------------------------------------ */

export class LocalVaultAdapter implements ISecretsPort {
  private readonly vaultPath: string;
  private readonly password: string;
  private readonly kdfIterations: number;

  constructor(vaultPath: string, password: string, kdfIterations?: number) {
    this.vaultPath = vaultPath;
    this.password = password;
    this.kdfIterations = kdfIterations ?? DEFAULT_KDF_ITERATIONS;
  }

  /* ----- Static factory ------------------------------------------- */

  /** Create a new encrypted vault file at `path`. */
  static createVault(path: string, password: string, kdfIterations?: number): void {
    const iterations = kdfIterations ?? DEFAULT_KDF_ITERATIONS;
    const salt = randomBytes(SALT_BYTES);
    const iv = randomBytes(IV_BYTES);
    const key = deriveKey(password, salt, iterations);
    const payload: VaultPayload = { version: 1, secrets: {} };
    const { ciphertext, tag } = encrypt(key, iv, JSON.stringify(payload));

    const envelope: VaultEnvelope = {
      salt: salt.toString('hex'),
      iv: iv.toString('hex'),
      tag: tag.toString('hex'),
      data: ciphertext.toString('hex'),
      kdf: 'pbkdf2',
      kdfIterations: iterations,
    };

    writeFileSync(path, JSON.stringify(envelope, null, 2), 'utf-8');
  }

  /* ----- ISecretsPort --------------------------------------------- */

  async resolveSecret(key: string): Promise<SecretResult> {
    const payload = this.readVault();
    const entry = payload.secrets[key];
    if (!entry) {
      return { ok: false, error: `Secret "${key}" not found in vault` };
    }
    return { ok: true, value: entry.value };
  }

  async hasSecret(key: string): Promise<boolean> {
    const payload = this.readVault();
    return key in payload.secrets;
  }

  async listSecrets(): Promise<SecretMetadata[]> {
    const payload = this.readVault();
    return Object.entries(payload.secrets).map(([key, entry]) => ({
      key,
      environment: 'local',
      createdAt: entry.createdAt,
      updatedAt: entry.updatedAt,
      version: entry.version,
    }));
  }

  /* ----- Mutation methods ----------------------------------------- */

  /** Add or update a secret. Re-encrypts the vault with a fresh IV. */
  addSecret(key: string, value: string): void {
    const payload = this.readVault();
    const now = new Date().toISOString();
    const existing = payload.secrets[key];

    payload.secrets[key] = {
      value,
      createdAt: existing?.createdAt ?? now,
      updatedAt: now,
      version: (existing?.version ?? 0) + 1,
    };

    this.writeVault(payload);
  }

  /** Remove a secret from the vault. Re-encrypts with a fresh IV. */
  removeSecret(key: string): void {
    const payload = this.readVault();
    delete payload.secrets[key];
    this.writeVault(payload);
  }

  /* ----- Private helpers ------------------------------------------ */

  private readVault(): VaultPayload {
    const raw = readFileSync(this.vaultPath, 'utf-8');
    const envelope: VaultEnvelope = JSON.parse(raw);
    const salt = Buffer.from(envelope.salt, 'hex');
    const iv = Buffer.from(envelope.iv, 'hex');
    const tag = Buffer.from(envelope.tag, 'hex');
    const ciphertext = Buffer.from(envelope.data, 'hex');
    const key = deriveKey(this.password, salt, envelope.kdfIterations);

    const plaintext = decrypt(key, iv, ciphertext, tag);
    return JSON.parse(plaintext) as VaultPayload;
  }

  private writeVault(payload: VaultPayload): void {
    // Read existing envelope to reuse salt
    const raw = readFileSync(this.vaultPath, 'utf-8');
    const envelope: VaultEnvelope = JSON.parse(raw);
    const salt = Buffer.from(envelope.salt, 'hex');

    // Fresh IV for every write
    const iv = randomBytes(IV_BYTES);
    const key = deriveKey(this.password, salt, this.kdfIterations);
    const { ciphertext, tag } = encrypt(key, iv, JSON.stringify(payload));

    const newEnvelope: VaultEnvelope = {
      salt: envelope.salt, // reuse
      iv: iv.toString('hex'),
      tag: tag.toString('hex'),
      data: ciphertext.toString('hex'),
      kdf: 'pbkdf2',
      kdfIterations: this.kdfIterations,
    };

    // Atomic write: tmp file then rename
    const tmpPath = `${this.vaultPath}.tmp`;
    writeFileSync(tmpPath, JSON.stringify(newEnvelope, null, 2), 'utf-8');
    renameSync(tmpPath, this.vaultPath);
  }
}

/* ------------------------------------------------------------------ */
/*  Crypto helpers (module-private)                                    */
/* ------------------------------------------------------------------ */

function deriveKey(password: string, salt: Buffer, iterations: number): Buffer {
  return pbkdf2Sync(password, salt, iterations, KEY_BYTES, 'sha512');
}

function encrypt(
  key: Buffer,
  iv: Buffer,
  plaintext: string,
): { ciphertext: Buffer; tag: Buffer } {
  const cipher = createCipheriv(ALGORITHM, key, iv, { authTagLength: AUTH_TAG_LENGTH });
  const encrypted = Buffer.concat([cipher.update(plaintext, 'utf-8'), cipher.final()]);
  const tag = cipher.getAuthTag();
  return { ciphertext: encrypted, tag };
}

function decrypt(
  key: Buffer,
  iv: Buffer,
  ciphertext: Buffer,
  tag: Buffer,
): string {
  const decipher = createDecipheriv(ALGORITHM, key, iv, { authTagLength: AUTH_TAG_LENGTH });
  decipher.setAuthTag(tag);
  const decrypted = Buffer.concat([decipher.update(ciphertext), decipher.final()]);
  return decrypted.toString('utf-8');
}
