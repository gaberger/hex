/**
 * project.ts — Project domain types (ADR-056).
 */

export interface Project {
  id: string;
  name: string;
  path: string;
  health?: 'green' | 'yellow' | 'red';
  lastActivity?: string;
}

export interface InitResult {
  initialized: boolean;
  name: string;
  path: string;
  created: string[];
}
