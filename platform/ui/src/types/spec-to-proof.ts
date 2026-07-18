import { z } from 'zod';

// TypeScript interfaces for domain models
export interface SpecDocument {
  id: string;
  contentSha256: string;
  sourceSystem: string;
  sourceId: string;
  title: string;
  content: string;
  url: string;
  author: string;
  createdAt: Date;
  modifiedAt: Date;
  metadata: Record<string, string>;
  version: number;
  status: DocumentStatus;
}

export interface Invariant {
  id: string;
  contentSha256: string;
  description: string;
  formalExpression: string;
  naturalLanguage: string;
  variables: Variable[];
  units: Record<string, string>;
  confidenceScore: number;
  sourceDocumentId: string;
  extractedAt: Date;
  status: InvariantStatus;
  tags: string[];
  priority: Priority;
}

export interface Variable {
  name: string;
  type: string;
  description: string;
  unit: string;
  constraints: string[];
}

export interface InvariantSet {
  id: string;
  contentSha256: string;
  name: string;
  description: string;
  invariants: Invariant[];
  sourceDocumentIds: string[];
  createdAt: Date;
  modifiedAt: Date;
  status: InvariantSetStatus;
}

export interface LeanTheorem {
  id: string;
  contentSha256: string;
  theoremName: string;
  leanCode: string;
  sourceInvariantId: string;
  generatedAt: Date;
  status: TheoremStatus;
  compilationErrors: string[];
  proofStrategy: string;
  metadata: Record<string, string>;
}

export interface ProofArtifact {
  id: string;
  contentSha256: string;
  theoremId: string;
  invariantId: string;
  status: ProofStatus;
  attemptedAt: Date;
  durationMs: number;
  output: string;
  logs: string[];
  resourceUsage: ResourceUsage;
  proofStrategy: string;
  confidenceScore: number;
  metadata: Record<string, string>;
}

export interface ResourceUsage {
  cpuSeconds: number;
  memoryBytes: number;
  diskBytes: number;
  networkBytes: number;
}

export interface BadgeStatus {
  id: string;
  contentSha256: string;
  repoOwner: string;
  repoName: string;
  prNumber: number;
  commitSha: string;
  state: BadgeState;
  description: string;
  targetUrl: string;
  createdAt: Date;
  updatedAt: Date;
  proofArtifactIds: string[];
  coveragePercentage: number;
  invariantsProven: number;
  totalInvariants: number;
}

// Enums
export enum DocumentStatus {
  UNSPECIFIED = 'UNSPECIFIED',
  DRAFT = 'DRAFT',
  PUBLISHED = 'PUBLISHED',
  ARCHIVED = 'ARCHIVED',
}

export enum InvariantStatus {
  UNSPECIFIED = 'UNSPECIFIED',
  EXTRACTED = 'EXTRACTED',
  CONFIRMED = 'CONFIRMED',
  REJECTED = 'REJECTED',
  PROVEN = 'PROVEN',
  FAILED = 'FAILED',
}

export enum InvariantSetStatus {
  UNSPECIFIED = 'UNSPECIFIED',
  DRAFT = 'DRAFT',
  REVIEW = 'REVIEW',
  APPROVED = 'APPROVED',
  PROVEN = 'PROVEN',
  FAILED = 'FAILED',
}

export enum TheoremStatus {
  UNSPECIFIED = 'UNSPECIFIED',
  GENERATED = 'GENERATED',
  COMPILING = 'COMPILING',
  COMPILED = 'COMPILED',
  PROVING = 'PROVING',
  PROVEN = 'PROVEN',
  FAILED = 'FAILED',
}

export enum ProofStatus {
  UNSPECIFIED = 'UNSPECIFIED',
  PENDING = 'PENDING',
  RUNNING = 'RUNNING',
  SUCCESS = 'SUCCESS',
  FAILED = 'FAILED',
  TIMEOUT = 'TIMEOUT',
  ERROR = 'ERROR',
}

export enum BadgeState {
  UNSPECIFIED = 'UNSPECIFIED',
  PENDING = 'PENDING',
  SUCCESS = 'SUCCESS',
  FAILURE = 'FAILURE',
  ERROR = 'ERROR',
}

export enum Priority {
  UNSPECIFIED = 'UNSPECIFIED',
  LOW = 'LOW',
  MEDIUM = 'MEDIUM',
  HIGH = 'HIGH',
  CRITICAL = 'CRITICAL',
}

// Zod schemas for validation
export const DocumentStatusSchema = z.nativeEnum(DocumentStatus);
export const InvariantStatusSchema = z.nativeEnum(InvariantStatus);
export const InvariantSetStatusSchema = z.nativeEnum(InvariantSetStatus);
export const TheoremStatusSchema = z.nativeEnum(TheoremStatus);
export const ProofStatusSchema = z.nativeEnum(ProofStatus);
export const BadgeStateSchema = z.nativeEnum(BadgeState);
export const PrioritySchema = z.nativeEnum(Priority);

export const VariableSchema = z.object({
  name: z.string().min(1),
  type: z.string().min(1),
  description: z.string(),
  unit: z.string(),
  constraints: z.array(z.string()),
});

export const ResourceUsageSchema = z.object({
  cpuSeconds: z.number().nonnegative(),
  memoryBytes: z.number().nonnegative(),
  diskBytes: z.number().nonnegative(),
  networkBytes: z.number().nonnegative(),
});

export const SpecDocumentSchema = z.object({
  id: z.string().uuid(),
  contentSha256: z.string().regex(/^[a-fA-F0-9]{64}$/),
  sourceSystem: z.string().min(1),
  sourceId: z.string().min(1),
  title: z.string().min(1),
  content: z.string(),
  url: z.string().url(),
  author: z.string().min(1),
  createdAt: z.date(),
  modifiedAt: z.date(),
  metadata: z.record(z.string()),
  version: z.number().int().positive(),
  status: DocumentStatusSchema,
});

export const InvariantSchema = z.object({
  id: z.string().uuid(),
  contentSha256: z.string().regex(/^[a-fA-F0-9]{64}$/),
  description: z.string().min(1),
  formalExpression: z.string().min(1),
  naturalLanguage: z.string(),
  variables: z.array(VariableSchema),
  units: z.record(z.string()),
  confidenceScore: z.number().min(0).max(1),
  sourceDocumentId: z.string().uuid(),
  extractedAt: z.date(),
  status: InvariantStatusSchema,
  tags: z.array(z.string()),
  priority: PrioritySchema,
});

export const InvariantSetSchema = z.object({
  id: z.string().uuid(),
  contentSha256: z.string().regex(/^[a-fA-F0-9]{64}$/),
  name: z.string().min(1),
  description: z.string(),
  invariants: z.array(InvariantSchema),
  sourceDocumentIds: z.array(z.string().uuid()),
  createdAt: z.date(),
  modifiedAt: z.date(),
  status: InvariantSetStatusSchema,
});

export const LeanTheoremSchema = z.object({
  id: z.string().uuid(),
  contentSha256: z.string().regex(/^[a-fA-F0-9]{64}$/),
  theoremName: z.string().min(1),
  leanCode: z.string().min(1),
  sourceInvariantId: z.string().uuid(),
  generatedAt: z.date(),
  status: TheoremStatusSchema,
  compilationErrors: z.array(z.string()),
  proofStrategy: z.string(),
  metadata: z.record(z.string()),
});

export const ProofArtifactSchema = z.object({
  id: z.string().uuid(),
  contentSha256: z.string().regex(/^[a-fA-F0-9]{64}$/),
  theoremId: z.string().uuid(),
  invariantId: z.string().uuid(),
  status: ProofStatusSchema,
  attemptedAt: z.date(),
  durationMs: z.number().int().nonnegative(),
  output: z.string(),
  logs: z.array(z.string()),
  resourceUsage: ResourceUsageSchema,
  proofStrategy: z.string(),
  confidenceScore: z.number().min(0).max(1),
  metadata: z.record(z.string()),
});

export const BadgeStatusSchema = z.object({
  id: z.string().uuid(),
  contentSha256: z.string().regex(/^[a-fA-F0-9]{64}$/),
  repoOwner: z.string().min(1),
  repoName: z.string().min(1),
  prNumber: z.number().int().positive(),
  commitSha: z.string().regex(/^[a-fA-F0-9]{40}$/),
  state: BadgeStateSchema,
  description: z.string(),
  targetUrl: z.string().url(),
  createdAt: z.date(),
  updatedAt: z.date(),
  proofArtifactIds: z.array(z.string().uuid()),
  coveragePercentage: z.number().min(0).max(100),
  invariantsProven: z.number().int().nonnegative(),
  totalInvariants: z.number().int().positive(),
});

// Utility functions

// SHA-256 round constants (first 32 bits of the fractional parts of the cube
// roots of the first 64 primes), per FIPS 180-4.
const SHA256_K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
  0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
  0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
  0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
  0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
  0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
  0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
  0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
  0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

const rotr = (value: number, bits: number): number =>
  (value >>> bits) | (value << (32 - bits));

/**
 * Compute the SHA-256 digest of `content` and return it as a lowercase hex
 * string.
 *
 * This is a self-contained implementation of FIPS 180-4. It is used instead of
 * the Web Crypto API (`crypto.subtle.digest`) because that API is asynchronous,
 * whereas this helper is consumed synchronously; the pure implementation also
 * runs identically in the browser, in Node, and under Jest without any runtime
 * dependency. The input is UTF-8 encoded before hashing so results match the
 * server-side Rust hashing (`sha2::Sha256` over the UTF-8 bytes).
 */
export function calculateSha256(content: string): string {
  const bytes = new TextEncoder().encode(content);

  // Pre-processing: append the bit '1', pad with zeros, then the 64-bit length.
  const bitLength = bytes.length * 8;
  const paddedLength = ((bytes.length + 8) >> 6 << 6) + 64;
  const buffer = new Uint8Array(paddedLength);
  buffer.set(bytes);
  buffer[bytes.length] = 0x80;
  // Write the message length as a big-endian 64-bit integer. JavaScript bitwise
  // ops are 32-bit, so the high word is derived via division.
  const view = new DataView(buffer.buffer);
  view.setUint32(paddedLength - 4, bitLength >>> 0, false);
  view.setUint32(paddedLength - 8, Math.floor(bitLength / 0x100000000), false);

  const h = new Uint32Array([
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c,
    0x1f83d9ab, 0x5be0cd19,
  ]);

  const w = new Uint32Array(64);

  for (let offset = 0; offset < paddedLength; offset += 64) {
    for (let i = 0; i < 16; i++) {
      w[i] = view.getUint32(offset + i * 4, false);
    }
    for (let i = 16; i < 64; i++) {
      const s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >>> 3);
      const s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >>> 10);
      w[i] = (w[i - 16] + s0 + w[i - 7] + s1) >>> 0;
    }

    let [a, b, c, d, e, f, g, hh] = h;

    for (let i = 0; i < 64; i++) {
      const s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
      const ch = (e & f) ^ (~e & g);
      const temp1 = (hh + s1 + ch + SHA256_K[i] + w[i]) >>> 0;
      const s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
      const maj = (a & b) ^ (a & c) ^ (b & c);
      const temp2 = (s0 + maj) >>> 0;

      hh = g;
      g = f;
      f = e;
      e = (d + temp1) >>> 0;
      d = c;
      c = b;
      b = a;
      a = (temp1 + temp2) >>> 0;
    }

    h[0] = (h[0] + a) >>> 0;
    h[1] = (h[1] + b) >>> 0;
    h[2] = (h[2] + c) >>> 0;
    h[3] = (h[3] + d) >>> 0;
    h[4] = (h[4] + e) >>> 0;
    h[5] = (h[5] + f) >>> 0;
    h[6] = (h[6] + g) >>> 0;
    h[7] = (h[7] + hh) >>> 0;
  }

  let hex = '';
  for (let i = 0; i < h.length; i++) {
    hex += h[i].toString(16).padStart(8, '0');
  }
  return hex;
}

export function generateId(): string {
  // Use a polyfill for crypto.randomUUID in test environments
  if (typeof crypto !== 'undefined' && crypto.randomUUID) {
    return crypto.randomUUID();
  }
  // Fallback for test environments
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, function(c) {
    const r = Math.random() * 16 | 0;
    const v = c === 'x' ? r : (r & 0x3 | 0x8);
    return v.toString(16);
  });
}

export function validateSpecDocument(doc: unknown): SpecDocument {
  return SpecDocumentSchema.parse(doc) as SpecDocument;
}

export function validateInvariant(invariant: unknown): Invariant {
  return InvariantSchema.parse(invariant) as Invariant;
}

export function validateInvariantSet(set: unknown): InvariantSet {
  return InvariantSetSchema.parse(set) as InvariantSet;
}

export function validateLeanTheorem(theorem: unknown): LeanTheorem {
  return LeanTheoremSchema.parse(theorem) as LeanTheorem;
}

export function validateProofArtifact(artifact: unknown): ProofArtifact {
  return ProofArtifactSchema.parse(artifact) as ProofArtifact;
}

export function validateBadgeStatus(badge: unknown): BadgeStatus {
  return BadgeStatusSchema.parse(badge) as BadgeStatus;
}

// Type guards
export function isSpecDocument(obj: unknown): obj is SpecDocument {
  return SpecDocumentSchema.safeParse(obj).success;
}

export function isInvariant(obj: unknown): obj is Invariant {
  return InvariantSchema.safeParse(obj).success;
}

export function isInvariantSet(obj: unknown): obj is InvariantSet {
  return InvariantSetSchema.safeParse(obj).success;
}

export function isLeanTheorem(obj: unknown): obj is LeanTheorem {
  return LeanTheoremSchema.safeParse(obj).success;
}

export function isProofArtifact(obj: unknown): obj is ProofArtifact {
  return ProofArtifactSchema.safeParse(obj).success;
}

export function isBadgeStatus(obj: unknown): obj is BadgeStatus {
  return BadgeStatusSchema.safeParse(obj).success;
}

// Conversion functions for API responses
export function fromApiSpecDocument(apiDoc: any): SpecDocument {
  return {
    id: apiDoc.id,
    contentSha256: apiDoc.content_sha256,
    sourceSystem: apiDoc.source_system,
    sourceId: apiDoc.source_id,
    title: apiDoc.title,
    content: apiDoc.content,
    url: apiDoc.url,
    author: apiDoc.author,
    createdAt: new Date(apiDoc.created_at),
    modifiedAt: new Date(apiDoc.modified_at),
    metadata: apiDoc.metadata || {},
    version: apiDoc.version,
    status: apiDoc.status as DocumentStatus,
  };
}

export function fromApiInvariant(apiInvariant: any): Invariant {
  return {
    id: apiInvariant.id,
    contentSha256: apiInvariant.content_sha256,
    description: apiInvariant.description,
    formalExpression: apiInvariant.formal_expression,
    naturalLanguage: apiInvariant.natural_language,
    variables: (apiInvariant.variables || []).map((v: any) => ({
      name: v.name,
      type: v.type,
      description: v.description,
      unit: v.unit,
      constraints: v.constraints || [],
    })),
    units: apiInvariant.units || {},
    confidenceScore: apiInvariant.confidence_score,
    sourceDocumentId: apiInvariant.source_document_id,
    extractedAt: new Date(apiInvariant.extracted_at),
    status: apiInvariant.status as InvariantStatus,
    tags: apiInvariant.tags || [],
    priority: apiInvariant.priority as Priority,
  };
}

export function toApiSpecDocument(doc: SpecDocument): any {
  return {
    id: doc.id,
    content_sha256: doc.contentSha256,
    source_system: doc.sourceSystem,
    source_id: doc.sourceId,
    title: doc.title,
    content: doc.content,
    url: doc.url,
    author: doc.author,
    created_at: doc.createdAt.toISOString(),
    modified_at: doc.modifiedAt.toISOString(),
    metadata: doc.metadata,
    version: doc.version,
    status: doc.status,
  };
}

export function toApiInvariant(invariant: Invariant): any {
  return {
    id: invariant.id,
    content_sha256: invariant.contentSha256,
    description: invariant.description,
    formal_expression: invariant.formalExpression,
    natural_language: invariant.naturalLanguage,
    variables: invariant.variables.map(v => ({
      name: v.name,
      type: v.type,
      description: v.description,
      unit: v.unit,
      constraints: v.constraints,
    })),
    units: invariant.units,
    confidence_score: invariant.confidenceScore,
    source_document_id: invariant.sourceDocumentId,
    extracted_at: invariant.extractedAt.toISOString(),
    status: invariant.status,
    tags: invariant.tags,
    priority: invariant.priority,
  };
} 