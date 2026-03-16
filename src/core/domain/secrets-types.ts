/**
 * Secrets Domain Value Objects
 *
 * Pure types for the secrets subsystem. No external dependencies.
 */

/** Context for scoping secret resolution (environment, project, path) */
export interface SecretContext {
  /** Infisical environment slug: dev, staging, prod, etc. */
  environment?: string;
  /** Infisical project ID (overrides default) */
  projectId?: string;
  /** Folder path within Infisical (e.g. "/api-keys") */
  path?: string;
}

/** Metadata about a stored secret (value intentionally excluded) */
export interface SecretMetadata {
  key: string;
  environment: string;
  createdAt: string;
  updatedAt: string;
  version: number;
}

/** Result of a secret resolution attempt */
export type SecretResult =
  | { ok: true; value: string }
  | { ok: false; error: string };
