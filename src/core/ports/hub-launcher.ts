/**
 * Hub Launcher Port
 *
 * Abstracts hex-hub daemon lifecycle management so primary adapters
 * (CLI, MCP) don't need to import the secondary adapter directly.
 */

export interface IHubLauncherPort {
  /** Locate the hex-hub binary on disk. Returns null if not installed. */
  findBinary(): string | null;

  /** Check if the hub daemon is currently running (health check). */
  isRunning(): Promise<boolean>;

  /** Start the hub daemon. Returns the URL it's listening on. */
  start(token?: string): Promise<{ started: boolean; url: string }>;

  /** Stop the hub daemon. Returns true if it was running. */
  stop(): Promise<boolean>;

  /** Get current daemon status. */
  status(): Promise<{ running: boolean; url: string | null; projects: number }>;
}
