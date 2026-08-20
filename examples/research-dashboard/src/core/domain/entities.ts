export interface CpuInfo {
  model: string;
  cores: number;
  loadAvg1: number;
  loadAvg5: number;
  loadAvg15: number;
}

export interface MemoryInfo {
  totalBytes: number;
  freeBytes: number;
  usedBytes: number;
}

export interface DiskInfo {
  mount: string;
  totalBytes: number;
  usedBytes: number;
  freeBytes: number;
}

export interface GpuInfo {
  name: string;
  memoryTotalMb: number;
  memoryUsedMb: number;
  memoryFreeMb: number;
  utilizationPct: number;
}

export interface SystemSnapshot {
  hostname: string;
  takenAt: string;
  cpu: CpuInfo;
  memory: MemoryInfo;
  disk: DiskInfo;
  gpus: GpuInfo[];
}

export type DocEntryKind = "file" | "dir";

export interface DocEntry {
  collection: string;
  relativePath: string;
  name: string;
  kind: DocEntryKind;
  sizeBytes: number;
  modifiedAt: string;
}

export interface DocContent {
  collection: string;
  relativePath: string;
  name: string;
  markdown: string;
}

export interface ChatSource {
  collection: string;
  relativePath: string;
  name: string;
  score: number;
}

export interface ChatAnswer {
  question: string;
  answer: string;
  sources: ChatSource[];
  answeredAt: string;
}
