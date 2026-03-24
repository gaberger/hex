import { describe, expect, it } from '@jest/globals';
import { createContainer } from '../../src/composition-root.js';

describe('Composition Root', () => {
  it('wires adapters to ports correctly', async () => {
    const container = createContainer();
    expect(container).toBeDefined();
    // Add more assertions as necessary based on the container's structure
  });

  it('resolves primary adapters', async () => {
    const container = createContainer();
    const primaryAdapter = container.primaryAdapter;
    expect(primaryAdapter).toBeDefined();
    // Specific assertions for primary adapter
  });

  it('resolves secondary adapters', async () => {
    const container = createContainer();
    const secondaryAdapter = container.secondaryAdapter;
    expect(secondaryAdapter).toBeDefined();
    // Specific assertions for secondary adapter
  });
});