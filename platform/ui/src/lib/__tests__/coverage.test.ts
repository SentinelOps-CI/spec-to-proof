import { describe, it, expect, beforeEach, afterEach } from '@jest/globals';
import {
  CoverageCalculator,
  CoverageMetrics,
  InMemoryCoverageDataSource,
  HttpCoverageDataSource,
  resolveCoverageDataSource,
  setCoverageDataSource,
} from '../coverage';

// Mock data for testing
const mockCoverageData = {
  overallStats: {
    repository: 'test-repo',
    totalSpecs: 100,
    provenSpecs: 85,
    coveragePercentage: 85.0,
    averageProofLatency: 45.2,
    lastUpdated: new Date(),
  },
  userStories: [
    {
      id: 'us-001',
      title: 'Test User Story 1',
      specs: [
        { id: 'spec-001', title: 'Test Spec 1', status: 'proven', proofLatency: 30 },
        { id: 'spec-002', title: 'Test Spec 2', status: 'proven', proofLatency: 25 },
        { id: 'spec-003', title: 'Test Spec 3', status: 'pending' },
      ],
      coveragePercentage: 66.7,
    },
    {
      id: 'us-002',
      title: 'Test User Story 2',
      specs: [
        { id: 'spec-004', title: 'Test Spec 4', status: 'proven', proofLatency: 40 },
        { id: 'spec-005', title: 'Test Spec 5', status: 'proven', proofLatency: 35 },
      ],
      coveragePercentage: 100.0,
    },
  ],
  trends: [
    { date: '2024-01-01', coveragePercentage: 80.0, newSpecs: 10, provenSpecs: 8 },
    { date: '2024-01-02', coveragePercentage: 85.0, newSpecs: 12, provenSpecs: 11 },
    { date: '2024-01-03', coveragePercentage: 90.0, newSpecs: 15, provenSpecs: 14 },
  ],
  topPerformers: [
    { userId: 'user-001', userName: 'Alice Johnson', specsProven: 45, averageLatency: 38.2 },
    { userId: 'user-002', userName: 'Bob Smith', specsProven: 38, averageLatency: 42.1 },
    { userId: 'user-003', userName: 'Carol Davis', specsProven: 32, averageLatency: 35.8 },
  ],
};

describe('CoverageCalculator', () => {
  describe('calculateCoveragePercentage', () => {
    it('should calculate correct coverage percentage', () => {
      const result = CoverageCalculator.calculateCoveragePercentage(85, 100);
      expect(result).toBe(85.0);
    });

    it('should return 0 for zero total specs', () => {
      const result = CoverageCalculator.calculateCoveragePercentage(0, 0);
      expect(result).toBe(0);
    });

    it('should handle decimal precision correctly', () => {
      const result = CoverageCalculator.calculateCoveragePercentage(83, 100);
      expect(result).toBe(83.0);
    });

    it('should round to 2 decimal places', () => {
      const result = CoverageCalculator.calculateCoveragePercentage(83.333, 100);
      expect(result).toBe(83.33);
    });
  });

  describe('calculateAverageLatency', () => {
    it('should calculate correct average latency', () => {
      const latencies = [30, 40, 50, 60];
      const result = CoverageCalculator.calculateAverageLatency(latencies);
      expect(result).toBe(45.0);
    });

    it('should return 0 for empty array', () => {
      const result = CoverageCalculator.calculateAverageLatency([]);
      expect(result).toBe(0);
    });

    it('should handle single value', () => {
      const result = CoverageCalculator.calculateAverageLatency([42.5]);
      expect(result).toBe(42.5);
    });

    it('should round to 2 decimal places', () => {
      const latencies = [30.123, 40.456, 50.789];
      const result = CoverageCalculator.calculateAverageLatency(latencies);
      expect(result).toBe(40.46);
    });
  });

  describe('calculateTrends', () => {
    it('should calculate trends with changes', () => {
      const historicalData = [
        { date: '2024-01-01', coverage: 80.0 },
        { date: '2024-01-02', coverage: 85.0 },
        { date: '2024-01-03', coverage: 90.0 },
      ];

      const result = CoverageCalculator.calculateTrends(historicalData);

      expect(result).toHaveLength(3);
      expect(result[0]).toEqual({
        date: '2024-01-01',
        coveragePercentage: 80.0,
        change: 0,
      });
      expect(result[1]).toEqual({
        date: '2024-01-02',
        coveragePercentage: 85.0,
        change: 5.0,
      });
      expect(result[2]).toEqual({
        date: '2024-01-03',
        coveragePercentage: 90.0,
        change: 5.0,
      });
    });

    it('should handle single data point', () => {
      const historicalData = [{ date: '2024-01-01', coverage: 80.0 }];
      const result = CoverageCalculator.calculateTrends(historicalData);

      expect(result).toHaveLength(1);
      expect(result[0]).toEqual({
        date: '2024-01-01',
        coveragePercentage: 80.0,
        change: 0,
      });
    });

    it('should handle negative changes', () => {
      const historicalData = [
        { date: '2024-01-01', coverage: 90.0 },
        { date: '2024-01-02', coverage: 85.0 },
      ];

      const result = CoverageCalculator.calculateTrends(historicalData);

      expect(result[1]).toEqual({
        date: '2024-01-02',
        coveragePercentage: 85.0,
        change: -5.0,
      });
    });
  });
});

describe('CoverageMetrics', () => {
  beforeEach(() => {
    CoverageMetrics.resetMetrics();
  });

  describe('recordApiCall', () => {
    it('should record successful API call', () => {
      CoverageMetrics.recordApiCall(150, true);
      CoverageMetrics.recordApiCall(200, true);

      const metrics = CoverageMetrics.getMetrics();
      expect(metrics.apiCalls).toBe(2);
      expect(metrics.averageResponseTime).toBe(175);
      expect(metrics.errors).toBe(0);
    });

    it('should record failed API call', () => {
      CoverageMetrics.recordApiCall(150, true);
      CoverageMetrics.recordApiCall(200, false);

      const metrics = CoverageMetrics.getMetrics();
      expect(metrics.apiCalls).toBe(2);
      expect(metrics.averageResponseTime).toBe(150);
      expect(metrics.errors).toBe(1);
    });

    it('should calculate average response time correctly', () => {
      CoverageMetrics.recordApiCall(100, true);
      CoverageMetrics.recordApiCall(200, true);
      CoverageMetrics.recordApiCall(300, true);

      const metrics = CoverageMetrics.getMetrics();
      expect(metrics.averageResponseTime).toBe(200);
    });
  });

  describe('getMetrics', () => {
    it('should return current metrics', () => {
      CoverageMetrics.recordApiCall(100, true);
      CoverageMetrics.recordApiCall(200, false);

      const metrics = CoverageMetrics.getMetrics();
      expect(metrics).toEqual({
        apiCalls: 2,
        averageResponseTime: 100,
        errors: 1,
      });
    });

    it('should return a copy of metrics', () => {
      CoverageMetrics.recordApiCall(100, true);
      const metrics1 = CoverageMetrics.getMetrics();
      const metrics2 = CoverageMetrics.getMetrics();

      expect(metrics1).toEqual(metrics2);
      expect(metrics1).not.toBe(metrics2); // Should be different objects
    });
  });

  describe('resetMetrics', () => {
    it('should reset all metrics to zero', () => {
      CoverageMetrics.recordApiCall(100, true);
      CoverageMetrics.recordApiCall(200, false);

      CoverageMetrics.resetMetrics();

      const metrics = CoverageMetrics.getMetrics();
      expect(metrics).toEqual({
        apiCalls: 0,
        averageResponseTime: 0,
        errors: 0,
      });
    });
  });
});

// Mock API response validation tests
describe('Coverage API Response Validation', () => {
  it('should validate overall stats structure', () => {
    const stats = mockCoverageData.overallStats;

    expect(stats).toHaveProperty('repository');
    expect(stats).toHaveProperty('totalSpecs');
    expect(stats).toHaveProperty('provenSpecs');
    expect(stats).toHaveProperty('coveragePercentage');
    expect(stats).toHaveProperty('averageProofLatency');
    expect(stats).toHaveProperty('lastUpdated');

    expect(typeof stats.repository).toBe('string');
    expect(typeof stats.totalSpecs).toBe('number');
    expect(typeof stats.provenSpecs).toBe('number');
    expect(typeof stats.coveragePercentage).toBe('number');
    expect(typeof stats.averageProofLatency).toBe('number');
    expect(stats.lastUpdated).toBeInstanceOf(Date);
  });

  it('should validate user stories structure', () => {
    const userStories = mockCoverageData.userStories;

    expect(Array.isArray(userStories)).toBe(true);

    userStories.forEach(story => {
      expect(story).toHaveProperty('id');
      expect(story).toHaveProperty('title');
      expect(story).toHaveProperty('specs');
      expect(story).toHaveProperty('coveragePercentage');

      expect(typeof story.id).toBe('string');
      expect(typeof story.title).toBe('string');
      expect(Array.isArray(story.specs)).toBe(true);
      expect(typeof story.coveragePercentage).toBe('number');

      story.specs.forEach(spec => {
        expect(spec).toHaveProperty('id');
        expect(spec).toHaveProperty('title');
        expect(spec).toHaveProperty('status');

        expect(typeof spec.id).toBe('string');
        expect(typeof spec.title).toBe('string');
        expect(['proven', 'pending', 'failed']).toContain(spec.status);

        if (spec.proofLatency !== undefined) {
          expect(typeof spec.proofLatency).toBe('number');
        }
      });
    });
  });

  it('should validate trends structure', () => {
    const trends = mockCoverageData.trends;

    expect(Array.isArray(trends)).toBe(true);

    trends.forEach(trend => {
      expect(trend).toHaveProperty('date');
      expect(trend).toHaveProperty('coveragePercentage');
      expect(trend).toHaveProperty('newSpecs');
      expect(trend).toHaveProperty('provenSpecs');

      expect(typeof trend.date).toBe('string');
      expect(typeof trend.coveragePercentage).toBe('number');
      expect(typeof trend.newSpecs).toBe('number');
      expect(typeof trend.provenSpecs).toBe('number');
    });
  });

  it('should validate top performers structure', () => {
    const performers = mockCoverageData.topPerformers;

    expect(Array.isArray(performers)).toBe(true);

    performers.forEach(performer => {
      expect(performer).toHaveProperty('userId');
      expect(performer).toHaveProperty('userName');
      expect(performer).toHaveProperty('specsProven');
      expect(performer).toHaveProperty('averageLatency');

      expect(typeof performer.userId).toBe('string');
      expect(typeof performer.userName).toBe('string');
      expect(typeof performer.specsProven).toBe('number');
      expect(typeof performer.averageLatency).toBe('number');
    });
  });
});

// Performance tests
describe('Coverage API Performance', () => {
  it('should handle large datasets efficiently', () => {
    const startTime = performance.now();

    // Simulate processing large dataset
    const largeDataset = Array.from({ length: 10000 }, (_, i) => ({
      id: `spec-${i}`,
      title: `Spec ${i}`,
      status: i % 3 === 0 ? 'proven' : i % 3 === 1 ? 'pending' : 'failed',
      proofLatency: i % 3 === 0 ? Math.random() * 100 : undefined,
    }));

    const provenSpecs = largeDataset.filter(spec => spec.status === 'proven').length;
    const totalSpecs = largeDataset.length;
    const coverage = CoverageCalculator.calculateCoveragePercentage(provenSpecs, totalSpecs);

    const endTime = performance.now();
    const processingTime = endTime - startTime;

    expect(coverage).toBeGreaterThan(0);
    expect(coverage).toBeLessThanOrEqual(100);
    expect(processingTime).toBeLessThan(100); // Should complete within 100ms
  });

  it('should handle concurrent API calls', async () => {
    const concurrentCalls = 10;
    const promises = [];

    for (let i = 0; i < concurrentCalls; i++) {
      promises.push(
        new Promise(resolve => {
          const startTime = performance.now();
          CoverageMetrics.recordApiCall(Math.random() * 200 + 50, Math.random() > 0.1);
          const endTime = performance.now();
          resolve(endTime - startTime);
        })
      );
    }

    const results = await Promise.all(promises);
    const averageTime = results.reduce((sum, time) => sum + time, 0) / results.length;

    expect(averageTime).toBeLessThan(10); // Each call should complete within 10ms
    expect(CoverageMetrics.getMetrics().apiCalls).toBe(concurrentCalls);
  });
});

// Error handling tests
describe('Coverage API Error Handling', () => {
  it('should handle invalid coverage percentage calculations', () => {
    expect(() => CoverageCalculator.calculateCoveragePercentage(-1, 100)).not.toThrow();
    expect(() => CoverageCalculator.calculateCoveragePercentage(100, -1)).not.toThrow();
  });

  it('should handle invalid latency calculations', () => {
    expect(() => CoverageCalculator.calculateAverageLatency([-1, 0, 100])).not.toThrow();
    expect(() => CoverageCalculator.calculateAverageLatency([NaN, Infinity, -Infinity])).not.toThrow();
  });

  it('should handle empty trends data', () => {
    const result = CoverageCalculator.calculateTrends([]);
    expect(result).toEqual([]);
  });
});

describe('InMemoryCoverageDataSource', () => {
  const ds = new InMemoryCoverageDataSource();

  it('derives overall stats from the seeded user stories', async () => {
    const stats = await ds.getOverallStats({ repository: 'my-repo', timeRange: '30d' });
    expect(stats.repository).toBe('my-repo');
    expect(stats.totalSpecs).toBeGreaterThan(0);
    expect(stats.provenSpecs).toBeLessThanOrEqual(stats.totalSpecs);
    expect(stats.coveragePercentage).toBeGreaterThanOrEqual(0);
    expect(stats.lastUpdated).toBeInstanceOf(Date);
  });

  it('returns user stories and looks them up by id', async () => {
    const stories = await ds.getUserStories({});
    expect(stories.length).toBeGreaterThan(0);
    const found = await ds.getUserStory(stories[0].id);
    expect(found?.id).toBe(stories[0].id);
    expect(await ds.getUserStory('does-not-exist')).toBeNull();
  });

  it('limits top performers', async () => {
    const performers = await ds.getTopPerformers({ limit: 2 });
    expect(performers).toHaveLength(2);
  });
});

describe('resolveCoverageDataSource', () => {
  const originalApiUrl = process.env.COVERAGE_API_URL;
  const originalDemo = process.env.COVERAGE_ALLOW_DEMO_DATA;

  afterEach(() => {
    setCoverageDataSource(null);
    if (originalApiUrl === undefined) delete process.env.COVERAGE_API_URL;
    else process.env.COVERAGE_API_URL = originalApiUrl;
    if (originalDemo === undefined) delete process.env.COVERAGE_ALLOW_DEMO_DATA;
    else process.env.COVERAGE_ALLOW_DEMO_DATA = originalDemo;
  });

  it('throws NOT_IMPLEMENTED when nothing is configured', () => {
    delete process.env.COVERAGE_API_URL;
    delete process.env.COVERAGE_ALLOW_DEMO_DATA;
    expect(() => resolveCoverageDataSource()).toThrow(/not configured/i);
  });

  it('uses the in-memory source when demo data is explicitly allowed', () => {
    delete process.env.COVERAGE_API_URL;
    process.env.COVERAGE_ALLOW_DEMO_DATA = 'true';
    expect(resolveCoverageDataSource()).toBeInstanceOf(InMemoryCoverageDataSource);
  });

  it('prefers the HTTP backend when COVERAGE_API_URL is set', () => {
    process.env.COVERAGE_API_URL = 'https://api.example.com';
    expect(resolveCoverageDataSource()).toBeInstanceOf(HttpCoverageDataSource);
  });

  it('honours an explicit override', () => {
    const override = new InMemoryCoverageDataSource();
    setCoverageDataSource(override);
    expect(resolveCoverageDataSource()).toBe(override);
  });
});

describe('HttpCoverageDataSource', () => {
  beforeEach(() => CoverageMetrics.resetMetrics());

  it('parses and validates a well-formed trends payload', async () => {
    const payload = [
      { date: '2024-01-01', coveragePercentage: 80, newSpecs: 10, provenSpecs: 8 },
    ];
    const fetchImpl = jest.fn(async () =>
      new Response(JSON.stringify(payload), { status: 200, headers: { 'Content-Type': 'application/json' } }),
    ) as unknown as typeof fetch;

    const ds = new HttpCoverageDataSource('https://api.example.com', { fetchImpl });
    const trends = await ds.getTrends({ timeRange: '30d' });

    expect(trends).toEqual(payload);
    expect(fetchImpl).toHaveBeenCalledTimes(1);
    expect(CoverageMetrics.getMetrics().apiCalls).toBe(1);
    expect(CoverageMetrics.getMetrics().errors).toBe(0);
  });

  it('fails loudly on a malformed payload', async () => {
    const fetchImpl = jest.fn(async () =>
      new Response(JSON.stringify([{ date: 123 }]), { status: 200, headers: { 'Content-Type': 'application/json' } }),
    ) as unknown as typeof fetch;

    const ds = new HttpCoverageDataSource('https://api.example.com', { fetchImpl });
    await expect(ds.getTrends({ timeRange: '30d' })).rejects.toThrow(/malformed/i);
    expect(CoverageMetrics.getMetrics().errors).toBe(1);
  });

  it('surfaces non-2xx responses as errors', async () => {
    const fetchImpl = jest.fn(async () => new Response('nope', { status: 500 })) as unknown as typeof fetch;
    const ds = new HttpCoverageDataSource('https://api.example.com', { fetchImpl });
    await expect(ds.getOverallStats({ timeRange: '30d' })).rejects.toThrow(/status 500/i);
  });
}); 