/**
 * Vault Management Port
 *
 * Write operations for the local encrypted vault.
 * Used by `hex secrets` CLI commands. Separated from ISecretsPort
 * because read-only consumers (LLM key lookup, etc.) don't need CRUD.
 *
 * createVault is a factory operation — works even before a vault exists.
 * addSecret/removeSecret require an active vault (throw if none).
 */

export interface IVaultManagementPort {
  /** Create a new encrypted vault file. Always available. */
  createVault(path: string, password: string): void;

  /** Add or update a secret in the vault. Throws if no vault is open. */
  addSecret(key: string, value: string): void;

  /** Remove a secret from the vault. Throws if no vault is open. */
  removeSecret(key: string): void;
}
