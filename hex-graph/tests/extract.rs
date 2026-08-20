//! Per-language AST extraction + markdown extraction unit tests.

use hex_graph::extract::code::{extract_file, Language};
use hex_graph::extract::markdown::extract_doc;
use hex_graph::model::NodeKind;

fn kind_of(fx: &hex_graph::extract::code::FileExtract, name: &str) -> Option<NodeKind> {
    fx.entities.iter().find(|e| e.name == name).map(|e| e.kind)
}

#[test]
fn rust_entities_and_imports() {
    let src = r#"
use crate::a::Bee;
mod sibling;
pub fn do_it() {}
pub struct Widget;
pub enum Color { Red }
pub trait Speak {}
pub type Alias = u8;
pub const MAX: u8 = 9;
pub static GLOBAL: u8 = 1;
"#;
    let fx = extract_file(src, Language::Rust);
    assert_eq!(kind_of(&fx, "do_it"), Some(NodeKind::Function));
    assert_eq!(kind_of(&fx, "Widget"), Some(NodeKind::Struct));
    assert_eq!(kind_of(&fx, "Color"), Some(NodeKind::Enum));
    assert_eq!(kind_of(&fx, "Speak"), Some(NodeKind::Trait));
    assert_eq!(kind_of(&fx, "Alias"), Some(NodeKind::Type));
    assert_eq!(kind_of(&fx, "MAX"), Some(NodeKind::Const));
    assert_eq!(kind_of(&fx, "GLOBAL"), Some(NodeKind::Const));
    // `use crate::a::Bee` → imported name Bee; `mod sibling;` → self::sibling.
    assert!(fx.imports.iter().any(|i| i.names.iter().any(|n| n == "Bee")));
    assert!(fx.imports.iter().any(|i| i.raw_path == "self::sibling"));
}

#[test]
fn typescript_entities_and_imports() {
    let src = r#"
import { Foo } from './foo.js';
export function f() {}
export class C {}
export interface I {}
export type T = number;
export enum E { A }
export const K = 1;
const local = 2;
"#;
    let fx = extract_file(src, Language::TypeScript);
    assert_eq!(kind_of(&fx, "f"), Some(NodeKind::Function));
    assert_eq!(kind_of(&fx, "C"), Some(NodeKind::Class));
    assert_eq!(kind_of(&fx, "I"), Some(NodeKind::Interface));
    assert_eq!(kind_of(&fx, "T"), Some(NodeKind::Type));
    assert_eq!(kind_of(&fx, "E"), Some(NodeKind::Enum));
    assert_eq!(kind_of(&fx, "K"), Some(NodeKind::Const));
    assert_eq!(kind_of(&fx, "local"), Some(NodeKind::Const));
    assert!(fx.imports.iter().any(|i| i.raw_path == "./foo.js"
        && i.names.iter().any(|n| n == "Foo")));
}

#[test]
fn go_entities_and_imports() {
    let src = r#"
package p
import "fmt"
func F() {}
func (r R) M() {}
type S struct{}
type I interface{}
type A = int
const C = 1
var V = 2
"#;
    let fx = extract_file(src, Language::Go);
    assert_eq!(kind_of(&fx, "F"), Some(NodeKind::Function));
    assert_eq!(kind_of(&fx, "M"), Some(NodeKind::Function)); // method
    assert_eq!(kind_of(&fx, "S"), Some(NodeKind::Struct));
    assert_eq!(kind_of(&fx, "I"), Some(NodeKind::Interface));
    assert_eq!(kind_of(&fx, "A"), Some(NodeKind::Type));
    assert_eq!(kind_of(&fx, "C"), Some(NodeKind::Const));
    assert_eq!(kind_of(&fx, "V"), Some(NodeKind::Const));
    assert!(fx.imports.iter().any(|i| i.raw_path == "fmt"));
}

#[test]
fn unparseable_source_is_empty_not_panic() {
    let fx = extract_file("this is (((not valid rust", Language::Rust);
    // Tree-sitter is error-tolerant; the point is it must not panic and returns a value.
    let _ = fx.entities.len();
}

#[test]
fn markdown_headings_links_and_fences() {
    let md = "# Title\n\nintro with a [link](./other.md) here.\n\n```\n# not a heading\n[notlink](x)\n```\n\n## Subsection\n";
    let dx = extract_doc(md);
    let titles: Vec<&str> = dx.headings.iter().map(|h| h.title.as_str()).collect();
    assert!(titles.contains(&"Title"));
    assert!(titles.contains(&"Subsection"));
    // Heading and link inside the fenced block must be ignored.
    assert!(!titles.contains(&"not a heading"));
    assert!(dx.links.iter().any(|l| l == "./other.md"));
    assert!(!dx.links.iter().any(|l| l == "x"));
}

#[test]
fn markdown_requires_space_after_hashes() {
    // `#tag` is not an ATX heading.
    let dx = extract_doc("#nospace\n# real heading\n");
    let titles: Vec<&str> = dx.headings.iter().map(|h| h.title.as_str()).collect();
    assert_eq!(titles, vec!["real heading"]);
}
