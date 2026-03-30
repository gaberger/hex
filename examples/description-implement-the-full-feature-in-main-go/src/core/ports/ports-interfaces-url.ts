import { ShortCode } from '../domain/value-objects/short-code.js';
import { OriginalUrl } from '../domain/value-objects/original-url.js';
import { UrlMapping } from '../domain/entities/url-mapping.js';

/**
 * Port for storing and retrieving URL mappings
 */
export interface UrlStoragePort {
  /**
   * Store a new URL mapping
   * @param mapping The URL mapping to store
   * @returns Promise that resolves when storage is complete
   * @throws Error if storage fails
   */
  store(mapping: UrlMapping): Promise<void>;

  /**
   * Retrieve original URL by short code
   * @param shortCode The short code to look up
   * @returns Promise resolving to the original URL, or null if not found
   * @throws Error if retrieval fails
   */
  findByShortCode(shortCode: ShortCode): Promise<OriginalUrl | null>;

  /**
   * Check if a short code already exists
   * @param shortCode The short code to check
   * @returns Promise resolving to true if exists, false otherwise
   * @throws Error if check fails
   */
  exists(shortCode: ShortCode): Promise<boolean>;
}

/**
 * Port for generating short codes
 */
export interface ShortCodeGeneratorPort {
  /**
   * Generate a unique short code
   * @returns Promise resolving to a new short code
   * @throws Error if generation fails
   */
  generate(): Promise<ShortCode>;
}

/**
 * Port for validating URLs
 */
export interface UrlValidatorPort {
  /**
   * Validate if a URL is reachable and valid
   * @param url The URL to validate
   * @returns Promise resolving to true if valid, false otherwise
   */
  isValid(url: OriginalUrl): Promise<boolean>;
}