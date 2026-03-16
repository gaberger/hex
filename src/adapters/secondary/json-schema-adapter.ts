/**
 * JSON Schema Adapter — implements ISchemaPort for 'jsonschema' format.
 *
 * Parses JSON Schema documents, validates values against type definitions,
 * generates TypeScript declarations, and detects breaking changes between versions.
 */

import type {
  ISchemaPort,
  SchemaDefinition,
  SchemaValidationResult,
  SchemaValidationError,
  TypeMapping,
  SchemaBreakingChange,
} from '../../core/ports/cross-lang.js';
import type { Language, CodeUnit } from '../../core/ports/index.js';

interface JsonSchemaType {
  type?: string;
  properties?: Record<string, JsonSchemaType>;
  required?: string[];
  enum?: unknown[];
  items?: JsonSchemaType;
  $ref?: string;
}

type TypeStore = Map<string, JsonSchemaType>;

export class JsonSchemaAdapter implements ISchemaPort {
  private readonly types: TypeStore = new Map();

  async load(schema: SchemaDefinition): Promise<void> {
    if (schema.format !== 'jsonschema') {
      throw new Error(`Unsupported schema format: ${schema.format}. Only 'jsonschema' is supported.`);
    }
    const doc = JSON.parse(schema.content);
    const defs: Record<string, JsonSchemaType> = doc.$defs ?? doc.definitions ?? {};
    for (const [name, typeDef] of Object.entries(defs)) {
      this.types.set(name, typeDef);
    }
  }

  async validate<T>(typeName: string, value: T): Promise<SchemaValidationResult> {
    const schema = this.types.get(typeName);
    if (!schema) {
      return { valid: false, errors: [{ path: '', message: `Unknown type: ${typeName}`, expected: typeName, actual: 'undefined' }] };
    }
    const errors: SchemaValidationError[] = [];
    this.checkValue(schema, value, '', errors);
    return { valid: errors.length === 0, errors };
  }

  async getTypeMapping(typeName: string): Promise<TypeMapping> {
    if (!this.types.has(typeName)) {
      throw new Error(`Unknown type: ${typeName}`);
    }
    return { schemaName: typeName, languageTypes: { typescript: typeName } };
  }

  async listTypes(): Promise<string[]> {
    return [...this.types.keys()];
  }

  async generateTypes(targetLanguage: Language): Promise<CodeUnit> {
    if (targetLanguage !== 'typescript') {
      throw new Error(`Unsupported target language: ${targetLanguage}. Only 'typescript' is supported.`);
    }
    const exportNames: string[] = [];
    const lines: string[] = [];
    for (const [name, schema] of this.types) {
      exportNames.push(name);
      lines.push(`export interface ${name} {`);
      if (schema.properties) {
        const required = new Set(schema.required ?? []);
        for (const [field, fieldSchema] of Object.entries(schema.properties)) {
          const opt = required.has(field) ? '' : '?';
          lines.push(`  ${field}${opt}: ${this.toTsType(fieldSchema)};`);
        }
      }
      lines.push('}');
      lines.push('');
    }
    return {
      filePath: 'generated-types.ts',
      content: lines.join('\n'),
      language: 'typescript',
      astSummary: {
        filePath: 'generated-types.ts',
        language: 'typescript',
        level: 'L0',
        exports: exportNames.map((n) => ({ name: n, kind: 'interface' as const, line: 0, isDefault: false })),
        imports: [],
        dependencies: [],
        lineCount: lines.length,
        tokenEstimate: lines.join('\n').length / 4,
      },
    };
  }

  async diffSchemas(before: SchemaDefinition, after: SchemaDefinition): Promise<SchemaBreakingChange[]> {
    const parse = (s: SchemaDefinition): Record<string, JsonSchemaType> => {
      const doc = JSON.parse(s.content);
      return doc.$defs ?? doc.definitions ?? {};
    };
    const oldDefs = parse(before);
    const newDefs = parse(after);
    const changes: SchemaBreakingChange[] = [];

    for (const [typeName, oldSchema] of Object.entries(oldDefs)) {
      const newSchema = newDefs[typeName];
      if (!newSchema) {
        changes.push({ typeName, field: '', kind: 'removed', description: `Type '${typeName}' was removed` });
        continue;
      }
      this.diffType(typeName, oldSchema, newSchema, changes);
    }
    return changes;
  }

  // ── Private helpers ──────────────────────────────────────

  private checkValue(schema: JsonSchemaType, value: unknown, path: string, errors: SchemaValidationError[]): void {
    if (schema.enum) {
      if (!schema.enum.includes(value)) {
        errors.push({ path, message: 'Value not in enum', expected: `one of [${schema.enum.join(', ')}]`, actual: String(value) });
      }
      return;
    }
    if (schema.type === 'object') {
      if (typeof value !== 'object' || value === null || Array.isArray(value)) {
        errors.push({ path, message: 'Expected object', expected: 'object', actual: typeof value });
        return;
      }
      const obj = value as Record<string, unknown>;
      for (const req of schema.required ?? []) {
        if (!(req in obj)) {
          errors.push({ path: `${path}.${req}`, message: 'Missing required field', expected: req, actual: 'undefined' });
        }
      }
      if (schema.properties) {
        for (const [field, fieldSchema] of Object.entries(schema.properties)) {
          if (field in obj) {
            this.checkValue(fieldSchema, obj[field], `${path}.${field}`, errors);
          }
        }
      }
    } else if (schema.type === 'array') {
      if (!Array.isArray(value)) {
        errors.push({ path, message: 'Expected array', expected: 'array', actual: typeof value });
        return;
      }
      if (schema.items) {
        for (let i = 0; i < value.length; i++) {
          this.checkValue(schema.items, value[i], `${path}[${i}]`, errors);
        }
      }
    } else if (schema.type === 'string') {
      if (typeof value !== 'string') {
        errors.push({ path, message: 'Expected string', expected: 'string', actual: typeof value });
      }
    } else if (schema.type === 'number' || schema.type === 'integer') {
      if (typeof value !== 'number') {
        errors.push({ path, message: `Expected ${schema.type}`, expected: schema.type, actual: typeof value });
      }
    } else if (schema.type === 'boolean') {
      if (typeof value !== 'boolean') {
        errors.push({ path, message: 'Expected boolean', expected: 'boolean', actual: typeof value });
      }
    }
  }

  private toTsType(schema: JsonSchemaType): string {
    if (schema.$ref) {
      const parts = schema.$ref.split('/');
      return parts[parts.length - 1];
    }
    if (schema.enum) {
      return schema.enum.map((v) => (typeof v === 'string' ? `'${v}'` : String(v))).join(' | ');
    }
    switch (schema.type) {
      case 'string': return 'string';
      case 'number': case 'integer': return 'number';
      case 'boolean': return 'boolean';
      case 'array': return schema.items ? `${this.toTsType(schema.items)}[]` : 'unknown[]';
      case 'object': return 'Record<string, unknown>';
      default: return 'unknown';
    }
  }

  private diffType(typeName: string, old: JsonSchemaType, cur: JsonSchemaType, changes: SchemaBreakingChange[]): void {
    const oldProps = old.properties ?? {};
    const newProps = cur.properties ?? {};

    for (const field of Object.keys(oldProps)) {
      if (!(field in newProps)) {
        changes.push({ typeName, field, kind: 'removed', description: `Field '${field}' was removed from '${typeName}'` });
        continue;
      }
      if (oldProps[field].type && newProps[field].type && oldProps[field].type !== newProps[field].type) {
        changes.push({ typeName, field, kind: 'type-changed', description: `Field '${field}' changed from '${oldProps[field].type}' to '${newProps[field].type}'` });
      }
      if (oldProps[field].enum && newProps[field].enum) {
        for (const val of oldProps[field].enum!) {
          if (!newProps[field].enum!.includes(val)) {
            changes.push({ typeName, field, kind: 'enum-value-removed', description: `Enum value '${val}' removed from '${field}'` });
          }
        }
      }
    }
    const oldRequired = new Set(old.required ?? []);
    for (const req of cur.required ?? []) {
      if (!oldRequired.has(req)) {
        changes.push({ typeName, field: req, kind: 'required-added', description: `Field '${req}' became required in '${typeName}'` });
      }
    }
  }
}
