import { describe, it, expect, mock } from 'bun:test';
import { GreetUseCase } from './greet-usecase.js';
import type { IGreetingOutputPort } from '../ports/greeting-port.js';

describe('GreetUseCase', () => {
  it('creates a greeting and delivers it via the output port', async () => {
    const mockOutput: IGreetingOutputPort = {
      deliver: mock(() => Promise.resolve()),
    };

    const useCase = new GreetUseCase(mockOutput);
    const result = await useCase.greet('Alice');

    expect(result.recipient).toBe('Alice');
    expect(result.message).toContain('Alice');
    expect(mockOutput.deliver).toHaveBeenCalledTimes(1);
  });
});
