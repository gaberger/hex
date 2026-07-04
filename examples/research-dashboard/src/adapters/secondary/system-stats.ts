import { execFile } from "node:child_process";
import { hostname, cpus, loadavg, totalmem, freemem } from "node:os";
import { promisify } from "node:util";
import type { GpuInfo, SystemSnapshot } from "../../core/domain/entities.js";
import type { ISystemStatsReader } from "../../core/ports/index.js";

const execFileAsync = promisify(execFile);

async function readDisk(mount: string): Promise<{ totalBytes: number; usedBytes: number; freeBytes: number }> {
  try {
    const { stdout } = await execFileAsync("df", ["-k", "--output=size,used,avail", mount]);
    const line = stdout.trim().split("\n").at(-1) ?? "";
    const [sizeKb, usedKb, availKb] = line.trim().split(/\s+/).map(Number);
    return { totalBytes: sizeKb * 1024, usedBytes: usedKb * 1024, freeBytes: availKb * 1024 };
  } catch {
    return { totalBytes: 0, usedBytes: 0, freeBytes: 0 };
  }
}

async function readGpus(): Promise<GpuInfo[]> {
  try {
    const { stdout } = await execFileAsync("nvidia-smi", [
      "--query-gpu=name,memory.total,memory.used,memory.free,utilization.gpu",
      "--format=csv,noheader,nounits",
    ]);
    return stdout
      .trim()
      .split("\n")
      .filter((line) => line.length > 0)
      .map((line) => {
        const [name, memTotal, memUsed, memFree, util] = line.split(",").map((s) => s.trim());
        return {
          name,
          memoryTotalMb: Number(memTotal),
          memoryUsedMb: Number(memUsed),
          memoryFreeMb: Number(memFree),
          utilizationPct: Number(util),
        };
      });
  } catch {
    return [];
  }
}

export class SystemStatsReader implements ISystemStatsReader {
  constructor(private readonly diskMount: string = "/") {}

  async read(): Promise<SystemSnapshot> {
    const [disk, gpus] = await Promise.all([readDisk(this.diskMount), readGpus()]);
    const cpuList = cpus();
    const [loadAvg1, loadAvg5, loadAvg15] = loadavg();

    return {
      hostname: hostname(),
      takenAt: new Date().toISOString(),
      cpu: {
        model: cpuList[0]?.model ?? "unknown",
        cores: cpuList.length,
        loadAvg1,
        loadAvg5,
        loadAvg15,
      },
      memory: {
        totalBytes: totalmem(),
        freeBytes: freemem(),
        usedBytes: totalmem() - freemem(),
      },
      disk: { mount: this.diskMount, ...disk },
      gpus,
    };
  }
}
