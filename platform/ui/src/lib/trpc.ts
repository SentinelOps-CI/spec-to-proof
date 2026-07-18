import { createTRPCReact } from '@trpc/react-query';
import { httpBatchLink } from '@trpc/client';
import { QueryClient } from '@tanstack/react-query';
import { initTRPC } from '@trpc/server';
import { z } from 'zod';

// Initialize tRPC for server-side
const t = initTRPC.create();

// Export server-side procedures
export const createTRPCRouter = t.router;
export const publicProcedure = t.procedure;
export const protectedProcedure = t.procedure.use(({ ctx, next }) => {
  // Add authentication logic here if needed
  return next({ ctx });
});

// Define the tRPC router type
export type AppRouter = {
  invariant: {
    list: {
      input: {
        documentId?: string;
        status?: 'EXTRACTED' | 'CONFIRMED' | 'REJECTED';
        limit?: number;
        offset?: number;
      };
      output: {
        invariants: any[];
        total: number;
        hasMore: boolean;
      };
    };
    update: {
      input: {
        id: string;
        description?: string;
        formalExpression?: string;
        naturalLanguage?: string;
        status?: 'CONFIRMED' | 'REJECTED';
        tags?: string[];
        priority?: 'LOW' | 'MEDIUM' | 'HIGH' | 'CRITICAL';
      };
      output: any;
    };
    split: {
      input: {
        id: string;
        invariants: Array<{
          description: string;
          formalExpression: string;
          naturalLanguage: string;
          variables: Array<{
            name: string;
            type: string;
            description: string;
            unit: string;
            constraints: string[];
          }>;
          tags: string[];
          priority: 'LOW' | 'MEDIUM' | 'HIGH' | 'CRITICAL';
        }>;
      };
      output: {
        originalId: string;
        newInvariants: any[];
      };
    };
    rename: {
      input: {
        id: string;
        description: string;
      };
      output: any;
    };
  };
  invariantSet: {
    create: {
      input: {
        name: string;
        description: string;
        invariantIds: string[];
        sourceDocumentIds: string[];
      };
      output: any;
    };
    publish: {
      input: {
        id: string;
      };
      output: {
        success: boolean;
        message: string;
      };
    };
  };
};

// Create the tRPC client
export const trpc = createTRPCReact<AppRouter>();

// Create a query client
export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: (failureCount, error: any) => {
        // Don't retry on 4xx errors
        if (error?.data?.code >= 400 && error?.data?.code < 500) {
          return false;
        }
        return failureCount < 3;
      },
      staleTime: 5 * 60 * 1000, // 5 minutes
      gcTime: 10 * 60 * 1000, // 10 minutes
    },
    mutations: {
      retry: false,
    },
  },
});

// Create tRPC client configuration
export const trpcClient = trpc.createClient({
  links: [
    httpBatchLink({
      url: process.env.NEXT_PUBLIC_API_URL || 'http://localhost:3001/trpc',
      // Add headers for authentication if needed
      headers: () => {
        const token = typeof window !== 'undefined' ? localStorage.getItem('auth-token') : null;
        return token ? { Authorization: `Bearer ${token}` } : {};
      },
    }),
  ],
});

// Error handling utilities
export class TRPCError extends Error {
  constructor(
    message: string,
    public code: string,
    public statusCode: number,
    public data?: any
  ) {
    super(message);
    this.name = 'TRPCError';
  }
}

export function handleTRPCError(error: any): TRPCError {
  if (error?.data?.code) {
    return new TRPCError(
      error.data.message || 'An error occurred',
      error.data.code,
      error.data.httpStatus || 500,
      error.data
    );
  }
  
  return new TRPCError(
    error?.message || 'An unknown error occurred',
    'UNKNOWN_ERROR',
    500,
    error
  );
}
