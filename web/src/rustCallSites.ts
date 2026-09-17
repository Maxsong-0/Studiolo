import type { RustCallLinks } from './rustCallLinks';

export interface RustSourceLocation {
  path: string;
  line: number;
  column: number;
}

export interface RustSourceCoverage {
  path: string;
  bytes: number;
  attributes: number;
  conditional_attributes: number;
  unexpanded_macros: number;
  macro_definitions: number;
  external_modules: number;
}

export type RustContextKind =
  | 'file'
  | 'module'
  | 'impl'
  | 'trait'
  | 'function'
  | 'method'
  | 'closure'
  | 'async'
  | 'const'
  | 'static';
export type RustCallKind = 'path' | 'method' | 'expression' | 'macro';

export interface RustCallContext {
  id: string;
  parent: string | null;
  kind: RustContextKind;
  name: string | null;
  source: RustSourceLocation;
}

export interface RustCallSite {
  id: string;
  context: string;
  kind: RustCallKind;
  name: string | null;
  source: RustSourceLocation;
}

export interface RustCallGraph {
  files: RustSourceCoverage[];
  contexts: RustCallContext[];
  calls: RustCallSite[];
}

export interface RustCallSitesData {
  status:
    | 'disabled'
    | 'unbound'
    | 'unavailable'
    | 'invalid'
    | 'unsupported'
    | 'ready';
  observed_at: string | null;
  graph: RustCallGraph | null;
  links?: RustCallLinks | null;
  diagnostics: string[];
}

export const rustContextKinds: Record<RustContextKind, string> = {
  file: '文件根',
  module: '内联模块',
  impl: 'impl 块',
  trait: 'trait',
  function: '函数',
  method: '方法',
  closure: '闭包',
  async: 'async 块',
  const: 'const 上下文',
  static: 'static 上下文',
};
export const rustCallKinds: Record<RustCallKind, string> = {
  path: '标识路径调用',
  method: '方法调用',
  expression: '表达式调用',
  macro: '未展开宏调用',
};
