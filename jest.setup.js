require('@testing-library/jest-dom');

// Mock Next.js router
jest.mock('next/router', () => ({
  useRouter() {
    return {
      route: '/',
      pathname: '/',
      query: {},
      asPath: '/',
      push: jest.fn(),
      pop: jest.fn(),
      reload: jest.fn(),
      back: jest.fn(),
      prefetch: jest.fn().mockResolvedValue(undefined),
      beforePopState: jest.fn(),
      events: {
        on: jest.fn(),
        off: jest.fn(),
        emit: jest.fn(),
      },
      isFallback: false,
    };
  },
}));

// Mock Next.js image
jest.mock('next/image', () => ({
  __esModule: true,
  default: (props) => {
    // eslint-disable-next-line @next/next/no-img-element
    return require('react').createElement('img', props);
  },
}));

// Mock window.matchMedia
Object.defineProperty(window, 'matchMedia', {
  writable: true,
  value: jest.fn().mockImplementation(query => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: jest.fn(), // deprecated
    removeListener: jest.fn(), // deprecated
    addEventListener: jest.fn(),
    removeEventListener: jest.fn(),
    dispatchEvent: jest.fn(),
  })),
});

// Mock ResizeObserver
global.ResizeObserver = jest.fn().mockImplementation(() => ({
  observe: jest.fn(),
  unobserve: jest.fn(),
  disconnect: jest.fn(),
}));

// Mock IntersectionObserver
global.IntersectionObserver = jest.fn().mockImplementation(() => ({
  observe: jest.fn(),
  unobserve: jest.fn(),
  disconnect: jest.fn(),
}));

// Mock crypto.randomUUID for test environment
Object.defineProperty(global, 'crypto', {
  value: {
    randomUUID: () => {
      return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, function(c) {
        const r = Math.random() * 16 | 0;
        const v = c === 'x' ? r : (r & 0x3 | 0x8);
        return v.toString(16);
      });
    },
  },
});

// Mock tRPC client.
//
// Both the React `trpc` client and the server-side helpers (`createTRPCRouter`,
// `publicProcedure`, `protectedProcedure`) are stubbed here so that modules
// which construct routers at import time (e.g. `lib/coverage.ts`) can be
// evaluated in the test environment without a live tRPC runtime. This stub is
// test-only; production code uses the real `initTRPC` implementation.
const makeProcedureStub = () => {
  const builder = {
    input: () => builder,
    output: () => builder,
    use: () => builder,
    query: (fn) => fn,
    mutation: (fn) => fn,
  };
  return builder;
};

jest.mock('@/lib/trpc', () => ({
  createTRPCRouter: (routes) => routes,
  publicProcedure: makeProcedureStub(),
  protectedProcedure: makeProcedureStub(),
  trpc: {
    invariant: {
      list: {
        useQuery: jest.fn(() => ({
          data: { invariants: [], total: 0, hasMore: false },
          isLoading: false,
          error: null,
        })),
      },
      update: {
        useMutation: jest.fn(() => ({
          mutate: jest.fn(),
          isLoading: false,
          error: null,
        })),
      },
      split: {
        useMutation: jest.fn(() => ({
          mutate: jest.fn(),
          isLoading: false,
          error: null,
        })),
      },
      rename: {
        useMutation: jest.fn(() => ({
          mutate: jest.fn(),
          isLoading: false,
          error: null,
        })),
      },
      confirm: {
        useMutation: jest.fn(() => ({
          mutate: jest.fn(),
          isLoading: false,
          error: null,
        })),
      },
      reject: {
        useMutation: jest.fn(() => ({
          mutate: jest.fn(),
          isLoading: false,
          error: null,
        })),
      },
    },
  },
})); 