import { z } from 'zod';
import { createTRPCRouter, publicProcedure } from './trpc';
import { TRPCError } from '@trpc/server';

// Coverage data schemas
export const CoverageStatsSchema = z.object({
  repository: z.string(),
  totalSpecs: z.number(),
  provenSpecs: z.number(),
  coveragePercentage: z.number(),
  averageProofLatency: z.number(),
  lastUpdated: z.date(),
});

export const UserStoryCoverageSchema = z.object({
  id: z.string(),
  title: z.string(),
  specs: z.array(z.object({
    id: z.string(),
    title: z.string(),
    status: z.enum(['proven', 'pending', 'failed']),
    proofLatency: z.number().optional(),
  })),
  coveragePercentage: z.number(),
});

export const CoverageRequestSchema = z.object({
  repository: z.string().optional(),
  timeRange: z.enum(['1d', '7d', '30d', '90d']).default('30d'),
  includeUserStories: z.boolean().default(false),
});

export const TrendSchema = z.object({
  date: z.string(),
  coveragePercentage: z.number(),
  newSpecs: z.number(),
  provenSpecs: z.number(),
});

export const TopPerformerSchema = z.object({
  userId: z.string(),
  userName: z.string(),
  specsProven: z.number(),
  averageLatency: z.number(),
});

export const CoverageResponseSchema = z.object({
  overallStats: CoverageStatsSchema,
  userStories: z.array(UserStoryCoverageSchema).optional(),
  trends: z.array(TrendSchema),
  topPerformers: z.array(TopPerformerSchema),
});

export type TimeRange = '1d' | '7d' | '30d' | '90d';
export type CoverageStats = z.infer<typeof CoverageStatsSchema>;
export type UserStoryCoverage = z.infer<typeof UserStoryCoverageSchema>;
export type Trend = z.infer<typeof TrendSchema>;
export type TopPerformer = z.infer<typeof TopPerformerSchema>;
export type CoverageResponse = z.infer<typeof CoverageResponseSchema>;

// ---------------------------------------------------------------------------
// Data-source abstraction
//
// The coverage router talks to a `CoverageDataSource` rather than to any
// concrete backend. Two implementations are provided:
//
//   * `HttpCoverageDataSource` — the production path. It reads coverage facts
//     from the backend coverage API (the same service the tRPC client points
//     at via `NEXT_PUBLIC_API_URL`), validating every response against the zod
//     schemas above so malformed payloads fail loudly.
//   * `InMemoryCoverageDataSource` — a deterministic fixture for local
//     development, Storybook, and tests. It never reaches production unless it
//     is explicitly opted into (see `resolveCoverageDataSource`).
//
// This keeps demo data clearly quarantined while still giving the dashboard a
// real backend path.
// ---------------------------------------------------------------------------

export interface OverallStatsQuery {
  repository?: string | undefined;
  timeRange: TimeRange;
}

export interface TrendsQuery {
  repository?: string | undefined;
  timeRange: TimeRange;
}

export interface TopPerformersQuery {
  repository?: string | undefined;
  limit: number;
}

export interface CoverageDataSource {
  getOverallStats(query: OverallStatsQuery): Promise<CoverageStats>;
  getUserStories(query: { repository?: string | undefined }): Promise<UserStoryCoverage[]>;
  getUserStory(userStoryId: string): Promise<UserStoryCoverage | null>;
  getTrends(query: TrendsQuery): Promise<Trend[]>;
  getTopPerformers(query: TopPerformersQuery): Promise<TopPerformer[]>;
}

// ---------------------------------------------------------------------------
// In-memory implementation (development / tests only)
// ---------------------------------------------------------------------------

const DEMO_USER_STORIES: UserStoryCoverage[] = [
  {
    id: 'us-001',
    title: 'Implement drift detection',
    specs: [
      { id: 'spec-001', title: 'Jira webhook integration', status: 'proven', proofLatency: 32 },
      { id: 'spec-002', title: 'Confluence sync', status: 'proven', proofLatency: 28 },
      { id: 'spec-003', title: 'Google Docs support', status: 'pending' },
    ],
    coveragePercentage: 66.7,
  },
  {
    id: 'us-002',
    title: 'Coverage dashboard',
    specs: [
      { id: 'spec-004', title: 'REST API endpoints', status: 'proven', proofLatency: 41 },
      { id: 'spec-005', title: 'React dashboard', status: 'proven', proofLatency: 38 },
    ],
    coveragePercentage: 100.0,
  },
];

const DEMO_TRENDS: Trend[] = [
  { date: '2024-01-01', coveragePercentage: 85.0, newSpecs: 10, provenSpecs: 8 },
  { date: '2024-01-02', coveragePercentage: 87.0, newSpecs: 12, provenSpecs: 11 },
  { date: '2024-01-03', coveragePercentage: 90.0, newSpecs: 15, provenSpecs: 14 },
];

const DEMO_TOP_PERFORMERS: TopPerformer[] = [
  { userId: 'user-001', userName: 'Alice Johnson', specsProven: 45, averageLatency: 38.2 },
  { userId: 'user-002', userName: 'Bob Smith', specsProven: 38, averageLatency: 42.1 },
  { userId: 'user-003', userName: 'Carol Davis', specsProven: 32, averageLatency: 35.8 },
];

/**
 * Deterministic in-memory data source. Intended for local development and
 * automated tests; it fabricates plausible coverage numbers and must never be
 * used to back a production dashboard.
 */
export class InMemoryCoverageDataSource implements CoverageDataSource {
  constructor(
    private readonly seed: {
      userStories: UserStoryCoverage[];
      trends: Trend[];
      topPerformers: TopPerformer[];
    } = {
      userStories: DEMO_USER_STORIES,
      trends: DEMO_TREND_CLONE(),
      topPerformers: DEMO_TOP_PERFORMERS,
    },
  ) {}

  async getOverallStats(query: OverallStatsQuery): Promise<CoverageStats> {
    const totalSpecs = this.seed.userStories.reduce((n, us) => n + us.specs.length, 0) || 1250;
    const provenSpecs =
      this.seed.userStories.reduce(
        (n, us) => n + us.specs.filter((s) => s.status === 'proven').length,
        0,
      ) || 1125;
    const latencies = this.seed.userStories
      .flatMap((us) => us.specs)
      .map((s) => s.proofLatency)
      .filter((l): l is number => typeof l === 'number');

    return {
      repository: query.repository ?? 'spec-to-proof',
      totalSpecs,
      provenSpecs,
      coveragePercentage: CoverageCalculator.calculateCoveragePercentage(provenSpecs, totalSpecs),
      averageProofLatency: CoverageCalculator.calculateAverageLatency(latencies),
      lastUpdated: new Date(),
    };
  }

  async getUserStories(): Promise<UserStoryCoverage[]> {
    return this.seed.userStories;
  }

  async getUserStory(userStoryId: string): Promise<UserStoryCoverage | null> {
    return this.seed.userStories.find((us) => us.id === userStoryId) ?? null;
  }

  async getTrends(): Promise<Trend[]> {
    return this.seed.trends;
  }

  async getTopPerformers(query: TopPerformersQuery): Promise<TopPerformer[]> {
    return this.seed.topPerformers.slice(0, query.limit);
  }
}

function DEMO_TREND_CLONE(): Trend[] {
  return DEMO_TRENDS.map((t) => ({ ...t }));
}

// ---------------------------------------------------------------------------
// HTTP implementation (production path)
// ---------------------------------------------------------------------------

/**
 * Reads coverage facts from the backend coverage API. Every response is parsed
 * through the zod schemas so that a malformed or partial payload surfaces as a
 * clear error rather than corrupting the dashboard.
 */
export class HttpCoverageDataSource implements CoverageDataSource {
  constructor(
    private readonly baseUrl: string,
    private readonly options: { fetchImpl?: typeof fetch; timeoutMs?: number | undefined; authToken?: string | undefined } = {},
  ) {}

  private get fetchImpl(): typeof fetch {
    const f = this.options.fetchImpl ?? globalThis.fetch;
    if (!f) {
      throw new TRPCError({
        code: 'INTERNAL_SERVER_ERROR',
        message: 'No fetch implementation available for HttpCoverageDataSource',
      });
    }
    return f;
  }

  private async request<T>(path: string, params: Record<string, string | number | undefined>, schema: z.ZodType<T>): Promise<T> {
    const url = new URL(path.replace(/^\//, ''), this.baseUrl.endsWith('/') ? this.baseUrl : `${this.baseUrl}/`);
    for (const [key, value] of Object.entries(params)) {
      if (value !== undefined) url.searchParams.set(key, String(value));
    }

    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), this.options.timeoutMs ?? 10_000);
    const start = Date.now();

    // Only genuine transport failures (network error, timeout/abort) are wrapped
    // here; HTTP-status and schema-validation problems are raised explicitly
    // below so their typed errors propagate unchanged.
    let response: Response;
    try {
      response = await this.fetchImpl(url.toString(), {
        method: 'GET',
        headers: {
          Accept: 'application/json',
          ...(this.options.authToken ? { Authorization: `Bearer ${this.options.authToken}` } : {}),
        },
        signal: controller.signal,
      });
    } catch (error) {
      CoverageMetrics.recordApiCall(Date.now() - start, false);
      throw new TRPCError({
        code: 'INTERNAL_SERVER_ERROR',
        message: `Coverage API request to ${path} failed`,
        cause: error,
      });
    } finally {
      clearTimeout(timeout);
    }

    if (!response.ok) {
      CoverageMetrics.recordApiCall(Date.now() - start, false);
      throw new TRPCError({
        code: response.status === 404 ? 'NOT_FOUND' : 'INTERNAL_SERVER_ERROR',
        message: `Coverage API request to ${path} failed with status ${response.status}`,
      });
    }

    let json: unknown;
    try {
      json = await response.json();
    } catch (error) {
      CoverageMetrics.recordApiCall(Date.now() - start, false);
      throw new TRPCError({
        code: 'INTERNAL_SERVER_ERROR',
        message: `Coverage API returned a non-JSON payload for ${path}`,
        cause: error,
      });
    }

    const parsed = schema.safeParse(json);
    if (!parsed.success) {
      CoverageMetrics.recordApiCall(Date.now() - start, false);
      throw new TRPCError({
        code: 'INTERNAL_SERVER_ERROR',
        message: `Coverage API returned a malformed payload for ${path}: ${parsed.error.message}`,
      });
    }

    CoverageMetrics.recordApiCall(Date.now() - start, true);
    return parsed.data;
  }

  async getOverallStats(query: OverallStatsQuery): Promise<CoverageStats> {
    // The API returns `lastUpdated` as an ISO string; coerce to Date before validation.
    const wireSchema = CoverageStatsSchema.extend({ lastUpdated: z.coerce.date() });
    return this.request('/coverage/overall', { repository: query.repository, timeRange: query.timeRange }, wireSchema);
  }

  async getUserStories(query: { repository?: string | undefined }): Promise<UserStoryCoverage[]> {
    return this.request('/coverage/user-stories', { repository: query.repository }, z.array(UserStoryCoverageSchema));
  }

  async getUserStory(userStoryId: string): Promise<UserStoryCoverage | null> {
    return this.request(`/coverage/user-stories/${encodeURIComponent(userStoryId)}`, {}, UserStoryCoverageSchema.nullable());
  }

  async getTrends(query: TrendsQuery): Promise<Trend[]> {
    return this.request('/coverage/trends', { repository: query.repository, timeRange: query.timeRange }, z.array(TrendSchema));
  }

  async getTopPerformers(query: TopPerformersQuery): Promise<TopPerformer[]> {
    return this.request('/coverage/top-performers', { repository: query.repository, limit: query.limit }, z.array(TopPerformerSchema));
  }
}

// ---------------------------------------------------------------------------
// Data-source resolution
// ---------------------------------------------------------------------------

let overrideDataSource: CoverageDataSource | null = null;

/**
 * Override the data source used by the router. Primarily for tests and
 * Storybook; pass `null` to restore environment-based resolution.
 */
export function setCoverageDataSource(dataSource: CoverageDataSource | null): void {
  overrideDataSource = dataSource;
}

/**
 * Resolve the coverage data source using, in order of precedence:
 *   1. An explicit override registered via {@link setCoverageDataSource}.
 *   2. `COVERAGE_API_URL` — enables the real backend (production path).
 *   3. `COVERAGE_ALLOW_DEMO_DATA=true` — enables the in-memory demo fixture.
 * Otherwise it throws `NOT_IMPLEMENTED`, so a misconfigured production
 * deployment can never silently serve fabricated coverage numbers.
 */
export function resolveCoverageDataSource(): CoverageDataSource {
  if (overrideDataSource) return overrideDataSource;

  const apiUrl = process.env.COVERAGE_API_URL;
  if (apiUrl && apiUrl.trim().length > 0) {
    return new HttpCoverageDataSource(apiUrl, {
      timeoutMs: process.env.COVERAGE_API_TIMEOUT_MS ? Number(process.env.COVERAGE_API_TIMEOUT_MS) : undefined,
      authToken: process.env.COVERAGE_API_TOKEN,
    });
  }

  if (process.env.COVERAGE_ALLOW_DEMO_DATA === 'true') {
    return new InMemoryCoverageDataSource();
  }

  throw new TRPCError({
    code: 'NOT_IMPLEMENTED',
    message:
      'Coverage datasource is not configured. Set COVERAGE_API_URL to the backend coverage API, ' +
      'or COVERAGE_ALLOW_DEMO_DATA=true to serve in-memory demonstration data in development.',
  });
}

function wrapError(context: string, error: unknown): never {
  if (error instanceof TRPCError) throw error;
  throw new TRPCError({
    code: 'INTERNAL_SERVER_ERROR',
    message: context,
    cause: error,
  });
}

// Coverage router
export const coverageRouter = createTRPCRouter({
  getCoverage: publicProcedure
    .input(CoverageRequestSchema)
    .output(CoverageResponseSchema)
    .query(async ({ input }) => {
      try {
        const ds = resolveCoverageDataSource();
        const { repository, timeRange, includeUserStories } = input;

        const [overallStats, trends, topPerformers] = await Promise.all([
          ds.getOverallStats({ repository, timeRange }),
          ds.getTrends({ repository, timeRange }),
          ds.getTopPerformers({ repository, limit: 10 }),
        ]);

        const userStories = includeUserStories ? await ds.getUserStories({ repository }) : undefined;

        return { overallStats, userStories, trends, topPerformers };
      } catch (error) {
        return wrapError('Failed to fetch coverage data', error);
      }
    }),

  getRepositoryCoverage: publicProcedure
    .input(z.object({ repository: z.string(), timeRange: z.enum(['1d', '7d', '30d', '90d']).default('30d') }))
    .output(CoverageStatsSchema)
    .query(async ({ input }) => {
      try {
        const ds = resolveCoverageDataSource();
        return await ds.getOverallStats({ repository: input.repository, timeRange: input.timeRange });
      } catch (error) {
        return wrapError('Failed to fetch repository coverage', error);
      }
    }),

  getUserStoryCoverage: publicProcedure
    .input(z.object({ userStoryId: z.string() }))
    .output(UserStoryCoverageSchema)
    .query(async ({ input }) => {
      try {
        const ds = resolveCoverageDataSource();
        const userStory = await ds.getUserStory(input.userStoryId);
        if (!userStory) {
          throw new TRPCError({ code: 'NOT_FOUND', message: 'User story not found' });
        }
        return userStory;
      } catch (error) {
        return wrapError('Failed to fetch user story coverage', error);
      }
    }),

  getCoverageTrends: publicProcedure
    .input(z.object({
      repository: z.string().optional(),
      timeRange: z.enum(['1d', '7d', '30d', '90d']).default('30d'),
    }))
    .output(z.array(TrendSchema))
    .query(async ({ input }) => {
      try {
        const ds = resolveCoverageDataSource();
        return await ds.getTrends({ repository: input.repository, timeRange: input.timeRange });
      } catch (error) {
        return wrapError('Failed to fetch coverage trends', error);
      }
    }),

  getTopPerformers: publicProcedure
    .input(z.object({
      repository: z.string().optional(),
      limit: z.number().min(1).max(50).default(10),
    }))
    .output(z.array(TopPerformerSchema))
    .query(async ({ input }) => {
      try {
        const ds = resolveCoverageDataSource();
        return await ds.getTopPerformers({ repository: input.repository, limit: input.limit });
      } catch (error) {
        return wrapError('Failed to fetch top performers', error);
      }
    }),
});

// Coverage calculation utilities
export class CoverageCalculator {
  static calculateCoveragePercentage(provenSpecs: number, totalSpecs: number): number {
    if (totalSpecs === 0) return 0;
    return Math.round((provenSpecs / totalSpecs) * 100 * 100) / 100; // Round to 2 decimal places
  }

  static calculateAverageLatency(latencies: number[]): number {
    if (latencies.length === 0) return 0;
    const sum = latencies.reduce((acc, latency) => acc + latency, 0);
    return Math.round((sum / latencies.length) * 100) / 100; // Round to 2 decimal places
  }

  static calculateTrends(historicalData: Array<{ date: string; coverage: number }>): Array<{
    date: string;
    coveragePercentage: number;
    change: number;
  }> {
    return historicalData.map((data, index) => {
      const previous = index > 0 ? historicalData[index - 1] : undefined;
      const change = previous ? data.coverage - previous.coverage : 0;

      return {
        date: data.date,
        coveragePercentage: data.coverage,
        change,
      };
    });
  }
}

// Performance monitoring
export class CoverageMetrics {
  private static metrics = {
    apiCalls: 0,
    averageResponseTime: 0,
    errors: 0,
  };

  static recordApiCall(responseTime: number, success: boolean): void {
    this.metrics.apiCalls++;

    if (success) {
      this.metrics.averageResponseTime =
        (this.metrics.averageResponseTime * (this.metrics.apiCalls - 1) + responseTime) /
        this.metrics.apiCalls;
    } else {
      this.metrics.errors++;
    }
  }

  static getMetrics() {
    return { ...this.metrics };
  }

  static resetMetrics(): void {
    this.metrics = {
      apiCalls: 0,
      averageResponseTime: 0,
      errors: 0,
    };
  }
}
