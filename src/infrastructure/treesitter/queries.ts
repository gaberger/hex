/**
 * Tree-sitter S-expression queries for TypeScript AST extraction.
 *
 * Pure data module -- no runtime dependencies.
 * Each query uses tree-sitter capture syntax (@name) so the adapter
 * can pull named nodes from query matches.
 */

// ── L1 Queries (skeleton: names only) ─────────────────────

export const TS_L1_EXPORTS = `
(export_statement
  declaration: [
    (function_declaration name: (identifier) @name) @decl
    (class_declaration name: (type_identifier) @name) @decl
    (interface_declaration name: (type_identifier) @name) @decl
    (type_alias_declaration name: (type_identifier) @name) @decl
    (enum_declaration name: (identifier) @name) @decl
    (lexical_declaration
      (variable_declarator name: (identifier) @name)) @decl
  ]
) @export
`.trim();

export const TS_L1_IMPORTS = `
(import_statement
  source: (string) @source
  (import_clause
    [
      (named_imports
        (import_specifier name: (identifier) @name))
      (identifier) @name
      (namespace_import (identifier) @name)
    ]
  )
) @import
`.trim();

// ── L2 Queries (signatures: params + return types) ────────

export const TS_L2_FUNCTION_SIG = `
(function_declaration
  name: (identifier) @name
  parameters: (formal_parameters) @params
  return_type: (type_annotation)? @ret
) @func
`.trim();

export const TS_L2_METHOD_SIG = `
[
  (method_signature
    name: (property_identifier) @name
    parameters: (formal_parameters) @params
    return_type: (type_annotation)? @ret) @method
  (method_definition
    name: (property_identifier) @name
    parameters: (formal_parameters) @params
    return_type: (type_annotation)? @ret) @method
]
`.trim();

export const TS_L2_CLASS_MEMBERS = `
(class_declaration
  name: (type_identifier) @className
  body: (class_body) @body
) @class
`.trim();

export const TS_L2_INTERFACE_MEMBERS = `
(interface_declaration
  name: (type_identifier) @ifaceName
  body: (interface_body) @body
) @iface
`.trim();

// ── Kind mapping ──────────────────────────────────────────

/** Maps tree-sitter node types to ExportEntry.kind values. */
export const TS_NODE_KIND_MAP: Record<string, string> = {
  function_declaration: 'function',
  class_declaration: 'class',
  interface_declaration: 'interface',
  type_alias_declaration: 'type',
  enum_declaration: 'enum',
  lexical_declaration: 'const',
};
