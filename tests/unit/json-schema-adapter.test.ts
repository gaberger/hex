import { describe, it, expect } from 'bun:test';
import { JsonSchemaAdapter } from '../../src/adapters/secondary/json-schema-adapter.js';
import type { SchemaDefinition } from '../../src/core/ports/cross-lang.js';

function makeSchema(defs: Record<string, unknown>, version = '1.0'): SchemaDefinition {
  return { format: 'jsonschema', content: JSON.stringify({ $defs: defs }), version };
}

describe('JsonSchemaAdapter', () => {
  it('rejects non-jsonschema formats', async () => {
    const adapter = new JsonSchemaAdapter();
    await expect(adapter.load({ format: 'protobuf', content: '', version: '1' })).rejects.toThrow('Unsupported schema format');
  });

  it('loads types from $defs and lists them', async () => {
    const adapter = new JsonSchemaAdapter();
    await adapter.load(makeSchema({ User: { type: 'object', properties: { name: { type: 'string' } } } }));
    expect(await adapter.listTypes()).toEqual(['User']);
  });

  it('loads types from definitions key', async () => {
    const adapter = new JsonSchemaAdapter();
    const schema: SchemaDefinition = {
      format: 'jsonschema',
      content: JSON.stringify({ definitions: { Item: { type: 'object' } } }),
      version: '1',
    };
    await adapter.load(schema);
    expect(await adapter.listTypes()).toEqual(['Item']);
  });

  it('validates a correct object', async () => {
    const adapter = new JsonSchemaAdapter();
    await adapter.load(makeSchema({
      User: { type: 'object', properties: { name: { type: 'string' }, age: { type: 'number' } }, required: ['name'] },
    }));
    const result = await adapter.validate('User', { name: 'Alice', age: 30 });
    expect(result.valid).toBe(true);
    expect(result.errors).toHaveLength(0);
  });

  it('reports missing required field', async () => {
    const adapter = new JsonSchemaAdapter();
    await adapter.load(makeSchema({
      User: { type: 'object', properties: { name: { type: 'string' } }, required: ['name'] },
    }));
    const result = await adapter.validate('User', {});
    expect(result.valid).toBe(false);
    expect(result.errors[0].path).toContain('name');
  });

  it('reports type mismatch', async () => {
    const adapter = new JsonSchemaAdapter();
    await adapter.load(makeSchema({
      Cfg: { type: 'object', properties: { count: { type: 'number' } } },
    }));
    const result = await adapter.validate('Cfg', { count: 'not-a-number' });
    expect(result.valid).toBe(false);
    expect(result.errors[0].message).toContain('number');
  });

  it('validates enum values', async () => {
    const adapter = new JsonSchemaAdapter();
    await adapter.load(makeSchema({ Status: { enum: ['active', 'inactive'] } }));
    expect((await adapter.validate('Status', 'active')).valid).toBe(true);
    expect((await adapter.validate('Status', 'deleted')).valid).toBe(false);
  });

  it('returns error for unknown type', async () => {
    const adapter = new JsonSchemaAdapter();
    const result = await adapter.validate('Ghost', {});
    expect(result.valid).toBe(false);
  });

  it('generates TypeScript interfaces', async () => {
    const adapter = new JsonSchemaAdapter();
    await adapter.load(makeSchema({
      User: { type: 'object', properties: { name: { type: 'string' }, age: { type: 'number' } }, required: ['name'] },
    }));
    const unit = await adapter.generateTypes('typescript');
    expect(unit.filePath).toBe('generated-types.ts');
    expect(unit.content).toContain('export interface User');
    expect(unit.content).toContain('name: string');
    expect(unit.content).toContain('age?: number');
  });

  it('diffSchemas detects removed type', async () => {
    const adapter = new JsonSchemaAdapter();
    const before = makeSchema({ A: { type: 'object' }, B: { type: 'object' } });
    const after = makeSchema({ A: { type: 'object' } });
    const changes = await adapter.diffSchemas(before, after);
    expect(changes).toHaveLength(1);
    expect(changes[0].kind).toBe('removed');
    expect(changes[0].typeName).toBe('B');
  });

  it('diffSchemas detects type-changed', async () => {
    const adapter = new JsonSchemaAdapter();
    const before = makeSchema({ X: { type: 'object', properties: { v: { type: 'string' } } } });
    const after = makeSchema({ X: { type: 'object', properties: { v: { type: 'number' } } } });
    const changes = await adapter.diffSchemas(before, after);
    expect(changes.some((c) => c.kind === 'type-changed' && c.field === 'v')).toBe(true);
  });

  it('diffSchemas detects required-added', async () => {
    const adapter = new JsonSchemaAdapter();
    const before = makeSchema({ X: { type: 'object', properties: { a: { type: 'string' } } } });
    const after = makeSchema({ X: { type: 'object', properties: { a: { type: 'string' } }, required: ['a'] } });
    const changes = await adapter.diffSchemas(before, after);
    expect(changes.some((c) => c.kind === 'required-added')).toBe(true);
  });

  it('diffSchemas detects enum-value-removed', async () => {
    const adapter = new JsonSchemaAdapter();
    const before = makeSchema({ S: { type: 'object', properties: { s: { enum: ['a', 'b', 'c'] } } } });
    const after = makeSchema({ S: { type: 'object', properties: { s: { enum: ['a', 'c'] } } } });
    const changes = await adapter.diffSchemas(before, after);
    expect(changes.some((c) => c.kind === 'enum-value-removed')).toBe(true);
  });

  it('getTypeMapping returns typescript mapping', async () => {
    const adapter = new JsonSchemaAdapter();
    await adapter.load(makeSchema({ Foo: { type: 'object' } }));
    const mapping = await adapter.getTypeMapping('Foo');
    expect(mapping.schemaName).toBe('Foo');
    expect(mapping.languageTypes.typescript).toBe('Foo');
  });
});
