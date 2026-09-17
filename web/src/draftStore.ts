import { byteLength } from './canonicalModel';
import type { CanonicalDocument, PendingMutation } from './canonicalModel';

export interface DraftRecord {
  base: CanonicalDocument;
  draft: string;
  pending: PendingMutation | null;
}
const size = (record: DraftRecord) =>
  [
    ...new Set([
      record.base.content,
      record.draft,
      record.pending?.content ?? '',
    ]),
  ].reduce((total, content) => total + byteLength(content), 0);

/** 只活在单个 SessionBoundary 内；锁定时不暴露正文，绝不持久化或逐出未保存稿。 */
export class DraftStore {
  constructor(
    private records = new Map<string, DraftRecord>(),
    private listeners = new Set<() => void>(),
    readonly ownerScope: string | null = null,
    private origin = window.location.origin,
    private lifetime = { active: true },
  ) {}
  forScope(scope: string | null) {
    return new DraftStore(
      this.records,
      this.listeners,
      scope,
      this.origin,
      this.lifetime,
    );
  }
  activate() {
    this.lifetime.active = true;
  }
  destroy() {
    this.lifetime.active = false;
    this.records.clear();
    this.listeners.forEach((listener) => listener());
  }
  private key(document: CanonicalDocument) {
    return JSON.stringify([
      this.ownerScope,
      this.origin,
      document.vault_uuid,
      document.path,
    ]);
  }
  private belongs(id: string) {
    return (
      this.ownerScope !== null &&
      id.startsWith(
        JSON.stringify([this.ownerScope, this.origin]).slice(0, -1) + ',',
      )
    );
  }
  hasAny = () => this.records.size > 0;
  getSnapshot = () => this.list().length;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  get(document: CanonicalDocument) {
    return this.ownerScope ? this.records.get(this.key(document)) : undefined;
  }
  list() {
    return [...this.records]
      .filter(([id]) => this.belongs(id))
      .map(([, record]) => record);
  }
  canKeep(record: DraftRecord) {
    if (!this.ownerScope || !this.lifetime.active) return false;
    const id = this.key(record.base);
    if (!this.records.has(id) && this.records.size >= 8) return false;
    const bytes = [...this.records].reduce(
      (total, [other, value]) => total + (other === id ? 0 : size(value)),
      size(record),
    );
    return bytes <= 4 * 1024 * 1024;
  }
  keep(record: DraftRecord) {
    if (!this.canKeep(record)) return false;
    this.records.set(this.key(record.base), record);
    this.listeners.forEach((listener) => listener());
    return true;
  }
  remove(document: CanonicalDocument) {
    this.records.delete(this.key(document));
    this.listeners.forEach((listener) => listener());
  }
  clear() {
    for (const id of this.records.keys())
      if (!this.ownerScope || this.belongs(id)) this.records.delete(id);
    this.listeners.forEach((listener) => listener());
  }
}
