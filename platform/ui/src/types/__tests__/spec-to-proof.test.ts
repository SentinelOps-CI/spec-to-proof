import { validateSpecDocument, validateInvariant, validateInvariantSet, validateLeanTheorem, validateProofArtifact, validateBadgeStatus, fromApiSpecDocument, fromApiInvariant, toApiSpecDocument, toApiInvariant, calculateSha256, generateId, isSpecDocument, isInvariant, isInvariantSet, isLeanTheorem, isProofArtifact, isBadgeStatus } from '../spec-to-proof';
import { SpecDocument, Invariant, InvariantSet, LeanTheorem, ProofArtifact, BadgeStatus, DocumentStatus, InvariantStatus, InvariantSetStatus, TheoremStatus, ProofStatus, BadgeState, Priority } from '../spec-to-proof';

describe('Spec-to-Proof Domain Models', () => {
  describe('SpecDocument', () => {
    const validSpecDocument: SpecDocument = {
      id: '123e4567-e89b-12d3-a456-426614174000',
      contentSha256: 'a'.repeat(64),
      sourceSystem: 'jira',
      sourceId: 'PROJ-123',
      title: 'User Authentication Requirements',
      content: 'Users must authenticate with valid credentials.',
      url: 'https://example.com/jira/browse/PROJ-123',
      author: 'john.doe@example.com',
      createdAt: new Date('2024-01-01T00:00:00Z'),
      modifiedAt: new Date('2024-01-02T00:00:00Z'),
      metadata: { project: 'auth-service', priority: 'high' },
      version: 1,
      status: DocumentStatus.PUBLISHED,
    };

    it('should validate a valid SpecDocument', () => {
      expect(() => validateSpecDocument(validSpecDocument)).not.toThrow();
    });

    it('should reject invalid UUID', () => {
      const invalidDoc = { ...validSpecDocument, id: 'invalid-uuid' };
      expect(() => validateSpecDocument(invalidDoc)).toThrow();
    });

    it('should reject invalid SHA256', () => {
      const invalidDoc = { ...validSpecDocument, contentSha256: 'invalid-hash' };
      expect(() => validateSpecDocument(invalidDoc)).toThrow();
    });

    it('should reject invalid URL', () => {
      const invalidDoc = { ...validSpecDocument, url: 'not-a-url' };
      expect(() => validateSpecDocument(invalidDoc)).toThrow();
    });

    it('should reject negative version', () => {
      const invalidDoc = { ...validSpecDocument, version: -1 };
      expect(() => validateSpecDocument(invalidDoc)).toThrow();
    });

    it('should convert from API format', () => {
      const apiDoc = {
        id: validSpecDocument.id,
        content_sha256: validSpecDocument.contentSha256,
        source_system: validSpecDocument.sourceSystem,
        source_id: validSpecDocument.sourceId,
        title: validSpecDocument.title,
        content: validSpecDocument.content,
        url: validSpecDocument.url,
        author: validSpecDocument.author,
        created_at: validSpecDocument.createdAt.toISOString(),
        modified_at: validSpecDocument.modifiedAt.toISOString(),
        metadata: validSpecDocument.metadata,
        version: validSpecDocument.version,
        status: validSpecDocument.status,
      };

      const result = fromApiSpecDocument(apiDoc);
      expect(result).toEqual(validSpecDocument);
    });

    it('should convert to API format', () => {
      const result = toApiSpecDocument(validSpecDocument);
      expect(result.id).toBe(validSpecDocument.id);
      expect(result.content_sha256).toBe(validSpecDocument.contentSha256);
      expect(result.source_system).toBe(validSpecDocument.sourceSystem);
      expect(result.created_at).toBe(validSpecDocument.createdAt.toISOString());
    });

    it('should pass type guard', () => {
      expect(isSpecDocument(validSpecDocument)).toBe(true);
      expect(isSpecDocument({})).toBe(false);
    });
  });

  describe('Invariant', () => {
    const validInvariant: Invariant = {
      id: '123e4567-e89b-12d3-a456-426614174001',
      contentSha256: 'b'.repeat(64),
      description: 'User password must be at least 8 characters',
      formalExpression: 'length(password) >= 8',
      naturalLanguage: 'The password length must be greater than or equal to 8 characters',
      variables: [
        {
          name: 'password',
          type: 'string',
          description: 'User password',
          unit: '',
          constraints: ['non-empty', 'alphanumeric'],
        },
      ],
      units: { password: 'characters' },
      confidenceScore: 0.95,
      sourceDocumentId: '123e4567-e89b-12d3-a456-426614174000',
      extractedAt: new Date('2024-01-01T00:00:00Z'),
      status: InvariantStatus.EXTRACTED,
      tags: ['security', 'validation'],
      priority: Priority.HIGH,
    };

    it('should validate a valid Invariant', () => {
      expect(() => validateInvariant(validInvariant)).not.toThrow();
    });

    it('should reject invalid confidence score', () => {
      const invalidInvariant = { ...validInvariant, confidenceScore: 1.5 };
      expect(() => validateInvariant(invalidInvariant)).toThrow();
    });

    it('should reject empty description', () => {
      const invalidInvariant = { ...validInvariant, description: '' };
      expect(() => validateInvariant(invalidInvariant)).toThrow();
    });

    it('should convert from API format', () => {
      const apiInvariant = {
        id: validInvariant.id,
        content_sha256: validInvariant.contentSha256,
        description: validInvariant.description,
        formal_expression: validInvariant.formalExpression,
        natural_language: validInvariant.naturalLanguage,
        variables: validInvariant.variables.map(v => ({
          name: v.name,
          type: v.type,
          description: v.description,
          unit: v.unit,
          constraints: v.constraints,
        })),
        units: validInvariant.units,
        confidence_score: validInvariant.confidenceScore,
        source_document_id: validInvariant.sourceDocumentId,
        extracted_at: validInvariant.extractedAt.toISOString(),
        status: validInvariant.status,
        tags: validInvariant.tags,
        priority: validInvariant.priority,
      };

      const result = fromApiInvariant(apiInvariant);
      expect(result).toEqual(validInvariant);
    });

    it('should convert to API format', () => {
      const result = toApiInvariant(validInvariant);
      expect(result.id).toBe(validInvariant.id);
      expect(result.content_sha256).toBe(validInvariant.contentSha256);
      expect(result.description).toBe(validInvariant.description);
      expect(result.formal_expression).toBe(validInvariant.formalExpression);
    });

    it('should pass type guard', () => {
      expect(isInvariant(validInvariant)).toBe(true);
      expect(isInvariant({})).toBe(false);
    });
  });

  describe('InvariantSet', () => {
    const validInvariantSet: InvariantSet = {
      id: '123e4567-e89b-12d3-a456-426614174002',
      contentSha256: 'c'.repeat(64),
      name: 'Authentication Requirements',
      description: 'All authentication-related invariants',
      invariants: [],
      sourceDocumentIds: ['123e4567-e89b-12d3-a456-426614174000'],
      createdAt: new Date('2024-01-01T00:00:00Z'),
      modifiedAt: new Date('2024-01-02T00:00:00Z'),
      status: InvariantSetStatus.DRAFT,
    };

    it('should validate a valid InvariantSet', () => {
      expect(() => validateInvariantSet(validInvariantSet)).not.toThrow();
    });

    it('should reject empty name', () => {
      const invalidSet = { ...validInvariantSet, name: '' };
      expect(() => validateInvariantSet(invalidSet)).toThrow();
    });

    it('should pass type guard', () => {
      expect(isInvariantSet(validInvariantSet)).toBe(true);
      expect(isInvariantSet({})).toBe(false);
    });
  });

  describe('LeanTheorem', () => {
    const validLeanTheorem: LeanTheorem = {
      id: '123e4567-e89b-12d3-a456-426614174003',
      contentSha256: 'd'.repeat(64),
      theoremName: 'password_length_theorem',
      leanCode: 'theorem password_length_theorem : length password ≥ 8 := by simp',
      sourceInvariantId: '123e4567-e89b-12d3-a456-426614174001',
      generatedAt: new Date('2024-01-01T00:00:00Z'),
      status: TheoremStatus.GENERATED,
      compilationErrors: [],
      proofStrategy: 'simp',
      metadata: { complexity: 'low', estimated_time: '5s' },
    };

    it('should validate a valid LeanTheorem', () => {
      expect(() => validateLeanTheorem(validLeanTheorem)).not.toThrow();
    });

    it('should reject empty theorem name', () => {
      const invalidTheorem = { ...validLeanTheorem, theoremName: '' };
      expect(() => validateLeanTheorem(invalidTheorem)).toThrow();
    });

    it('should pass type guard', () => {
      expect(isLeanTheorem(validLeanTheorem)).toBe(true);
      expect(isLeanTheorem({})).toBe(false);
    });
  });

  describe('ProofArtifact', () => {
    const validProofArtifact: ProofArtifact = {
      id: '123e4567-e89b-12d3-a456-426614174004',
      contentSha256: 'e'.repeat(64),
      theoremId: '123e4567-e89b-12d3-a456-426614174003',
      invariantId: '123e4567-e89b-12d3-a456-426614174001',
      status: ProofStatus.SUCCESS,
      attemptedAt: new Date('2024-01-01T00:00:00Z'),
      durationMs: 5000,
      output: 'Proof completed successfully',
      logs: ['Compiling theorem...', 'Proof found'],
      resourceUsage: {
        cpuSeconds: 4.5,
        memoryBytes: 1024 * 1024 * 100,
        diskBytes: 1024 * 1024 * 10,
        networkBytes: 1024 * 512,
      },
      proofStrategy: 'simp',
      confidenceScore: 1.0,
      metadata: { proof_time: '5s', memory_peak: '100MB' },
    };

    it('should validate a valid ProofArtifact', () => {
      expect(() => validateProofArtifact(validProofArtifact)).not.toThrow();
    });

    it('should reject negative duration', () => {
      const invalidArtifact = { ...validProofArtifact, durationMs: -1 };
      expect(() => validateProofArtifact(invalidArtifact)).toThrow();
    });

    it('should pass type guard', () => {
      expect(isProofArtifact(validProofArtifact)).toBe(true);
      expect(isProofArtifact({})).toBe(false);
    });
  });

  describe('BadgeStatus', () => {
    const validBadgeStatus: BadgeStatus = {
      id: '123e4567-e89b-12d3-a456-426614174005',
      contentSha256: 'f'.repeat(64),
      repoOwner: 'fraware',
      repoName: 'spec-to-proof',
      prNumber: 123,
      commitSha: 'a'.repeat(40),
      state: BadgeState.SUCCESS,
      description: 'All invariants proven successfully',
      targetUrl: 'https://example.com/proofs/123',
      createdAt: new Date('2024-01-01T00:00:00Z'),
      updatedAt: new Date('2024-01-02T00:00:00Z'),
      proofArtifactIds: ['123e4567-e89b-12d3-a456-426614174004'],
      coveragePercentage: 100.0,
      invariantsProven: 5,
      totalInvariants: 5,
    };

    it('should validate a valid BadgeStatus', () => {
      expect(() => validateBadgeStatus(validBadgeStatus)).not.toThrow();
    });

    it('should reject invalid commit SHA', () => {
      const invalidBadge = { ...validBadgeStatus, commitSha: 'invalid-sha' };
      expect(() => validateBadgeStatus(invalidBadge)).toThrow();
    });

    it('should reject coverage > 100%', () => {
      const invalidBadge = { ...validBadgeStatus, coveragePercentage: 150.0 };
      expect(() => validateBadgeStatus(invalidBadge)).toThrow();
    });

    it('should pass type guard', () => {
      expect(isBadgeStatus(validBadgeStatus)).toBe(true);
      expect(isBadgeStatus({})).toBe(false);
    });
  });

  describe('Utility Functions', () => {
    it('should generate SHA256 hash', () => {
      const content = 'test content';
      const hash = calculateSha256(content);
      // Real SHA-256 digest, matching the server-side Rust `sha2::Sha256`
      // hashing of the same UTF-8 bytes.
      expect(hash).toBe('6ae8a75555209fd6c44157c0aed8016e763ff435a19cf186f76863140143ff72');
    });

    it('should match known SHA256 test vectors', () => {
      // FIPS 180-4 / NIST published vectors.
      expect(calculateSha256('')).toBe(
        'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'
      );
      expect(calculateSha256('abc')).toBe(
        'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
      );
    });

    it('should generate UUID', () => {
      const id1 = generateId();
      const id2 = generateId();
      expect(id1).not.toBe(id2);
      expect(id1).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/);
    });
  });

  // Additional validation tests
  describe('Additional Validation Tests', () => {
    it('should round-trip SpecDocument through API conversion', () => {
      const original: SpecDocument = {
        id: '123e4567-e89b-12d3-a456-426614174000',
        contentSha256: 'a'.repeat(64),
        sourceSystem: 'jira',
        sourceId: 'PROJ-123',
        title: 'Test Document',
        content: 'Test content',
        url: 'https://example.com/test',
        author: 'test@example.com',
        createdAt: new Date('2024-01-01T00:00:00Z'),
        modifiedAt: new Date('2024-01-02T00:00:00Z'),
        metadata: { test: 'value' },
        version: 1,
        status: DocumentStatus.PUBLISHED,
      };

      const apiFormat = toApiSpecDocument(original);
      const roundTrip = fromApiSpecDocument(apiFormat);

      expect(roundTrip).toEqual(original);
    });

    it('should round-trip Invariant through API conversion', () => {
      const original: Invariant = {
        id: '123e4567-e89b-12d3-a456-426614174001',
        contentSha256: 'b'.repeat(64),
        description: 'Test invariant',
        formalExpression: 'test > 0',
        naturalLanguage: 'Test value must be positive',
        variables: [
          {
            name: 'test',
            type: 'number',
            description: 'Test variable',
            unit: '',
            constraints: ['positive'],
          },
        ],
        units: { test: 'units' },
        confidenceScore: 0.95,
        sourceDocumentId: '123e4567-e89b-12d3-a456-426614174000',
        extractedAt: new Date('2024-01-01T00:00:00Z'),
        status: InvariantStatus.EXTRACTED,
        tags: ['test'],
        priority: Priority.HIGH,
      };

      const apiFormat = toApiInvariant(original);
      const roundTrip = fromApiInvariant(apiFormat);

      expect(roundTrip).toEqual(original);
    });

    it('should maintain SHA256 hash consistency', () => {
      const content = 'test content';
      const hash1 = calculateSha256(content);
      const hash2 = calculateSha256(content);
      expect(hash1).toBe(hash2);
    });

    it('should generate unique UUIDs', () => {
      const ids = Array.from({ length: 10 }, () => generateId());
      const uniqueIds = new Set(ids);
      expect(uniqueIds.size).toBe(10);
    });

    it('should validate SpecDocument with various data', () => {
      const validDocs = [
        {
          id: '123e4567-e89b-12d3-a456-426614174000',
          contentSha256: 'a'.repeat(64),
          sourceSystem: 'jira',
          sourceId: 'PROJ-123',
          title: 'Test Document 1',
          content: 'Test content 1',
          url: 'https://example.com/test1',
          author: 'test1@example.com',
          createdAt: new Date('2024-01-01T00:00:00Z'),
          modifiedAt: new Date('2024-01-02T00:00:00Z'),
          metadata: { test: 'value1' },
          version: 1,
          status: DocumentStatus.PUBLISHED,
        },
        {
          id: '123e4567-e89b-12d3-a456-426614174001',
          contentSha256: 'b'.repeat(64),
          sourceSystem: 'confluence',
          sourceId: 'CONF-456',
          title: 'Test Document 2',
          content: 'Test content 2',
          url: 'https://example.com/test2',
          author: 'test2@example.com',
          createdAt: new Date('2024-01-03T00:00:00Z'),
          modifiedAt: new Date('2024-01-04T00:00:00Z'),
          metadata: { test: 'value2' },
          version: 2,
          status: DocumentStatus.DRAFT,
        },
      ];

      validDocs.forEach(doc => {
        expect(() => validateSpecDocument(doc)).not.toThrow();
      });
    });

    it('should validate Invariant with various data', () => {
      const validInvariants = [
        {
          id: '123e4567-e89b-12d3-a456-426614174002',
          contentSha256: 'c'.repeat(64),
          description: 'Test invariant 1',
          formalExpression: 'x > 0',
          naturalLanguage: 'X must be positive',
          variables: [
            {
              name: 'x',
              type: 'number',
              description: 'Variable X',
              unit: '',
              constraints: ['positive'],
            },
          ],
          units: { x: 'units' },
          confidenceScore: 0.9,
          sourceDocumentId: '123e4567-e89b-12d3-a456-426614174000',
          extractedAt: new Date('2024-01-01T00:00:00Z'),
          status: InvariantStatus.EXTRACTED,
          tags: ['test', 'validation'],
          priority: Priority.HIGH,
        },
        {
          id: '123e4567-e89b-12d3-a456-426614174003',
          contentSha256: 'd'.repeat(64),
          description: 'Test invariant 2',
          formalExpression: 'y >= 10',
          naturalLanguage: 'Y must be greater than or equal to 10',
          variables: [
            {
              name: 'y',
              type: 'number',
              description: 'Variable Y',
              unit: '',
              constraints: ['non-negative'],
            },
          ],
          units: { y: 'units' },
          confidenceScore: 0.85,
          sourceDocumentId: '123e4567-e89b-12d3-a456-426614174001',
          extractedAt: new Date('2024-01-02T00:00:00Z'),
          status: InvariantStatus.CONFIRMED,
          tags: ['test', 'range'],
          priority: Priority.MEDIUM,
        },
      ];

      validInvariants.forEach(invariant => {
        expect(() => validateInvariant(invariant)).not.toThrow();
      });
    });

    it('should validate BadgeStatus with various data', () => {
      const validBadges = [
        {
          id: '123e4567-e89b-12d3-a456-426614174004',
          contentSha256: 'e'.repeat(64),
          repoOwner: 'testuser',
          repoName: 'testrepo',
          prNumber: 123,
          commitSha: 'a'.repeat(40),
          state: BadgeState.SUCCESS,
          description: 'All tests passing',
          targetUrl: 'https://example.com/badge/123',
          createdAt: new Date('2024-01-01T00:00:00Z'),
          updatedAt: new Date('2024-01-02T00:00:00Z'),
          proofArtifactIds: ['123e4567-e89b-12d3-a456-426614174005'],
          coveragePercentage: 100.0,
          invariantsProven: 5,
          totalInvariants: 5,
        },
        {
          id: '123e4567-e89b-12d3-a456-426614174006',
          contentSha256: 'f'.repeat(64),
          repoOwner: 'testuser2',
          repoName: 'testrepo2',
          prNumber: 456,
          commitSha: 'b'.repeat(40),
          state: BadgeState.FAILURE,
          description: 'Some tests failing',
          targetUrl: 'https://example.com/badge/456',
          createdAt: new Date('2024-01-03T00:00:00Z'),
          updatedAt: new Date('2024-01-04T00:00:00Z'),
          proofArtifactIds: ['123e4567-e89b-12d3-a456-426614174007'],
          coveragePercentage: 80.0,
          invariantsProven: 4,
          totalInvariants: 5,
        },
      ];

      validBadges.forEach(badge => {
        expect(() => validateBadgeStatus(badge)).not.toThrow();
      });
    });
  });
}); 