export interface CliPortInterface {
  handleCommand(command: string): Promise<void>;
}