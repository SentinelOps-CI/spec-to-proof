import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react';
import { InvariantCard } from '../InvariantCard';
import { Invariant, InvariantStatus, Priority } from '@/types/spec-to-proof';

// Mock tRPC. The card renders the split/rename modals (which register their own
// mutations at render time), so every mutation used by the component subtree
// must be stubbed, not just `update`.
jest.mock('@/lib/trpc', () => {
  const mutationStub = () => ({ mutate: jest.fn(), isLoading: false });
  return {
    trpc: {
      invariant: {
        update: { useMutation: mutationStub },
        split: { useMutation: mutationStub },
        rename: { useMutation: mutationStub },
        confirm: { useMutation: mutationStub },
        reject: { useMutation: mutationStub },
      },
    },
  };
});

// Mock toast
jest.mock('@/components/ui/Toast', () => ({
  toast: {
    success: jest.fn(),
    error: jest.fn(),
  },
}));

const mockInvariant: Invariant = {
  id: '123e4567-e89b-12d3-a456-426614174000',
  contentSha256: 'a'.repeat(64),
  description: 'Test invariant description',
  formalExpression: '∀x ∈ ℝ, x² ≥ 0',
  naturalLanguage: 'All real numbers squared are non-negative',
  variables: [
    {
      name: 'x',
      type: 'real',
      description: 'A real number',
      unit: '',
      constraints: ['x ∈ ℝ'],
    },
  ],
  units: {},
  confidenceScore: 0.95,
  sourceDocumentId: 'doc-123',
  // Local-time construction keeps `toLocaleDateString()` deterministic across
  // timezones (a UTC-midnight date would render as the previous day west of UTC).
  extractedAt: new Date(2024, 0, 1),
  status: InvariantStatus.EXTRACTED,
  tags: ['mathematics', 'algebra'],
  priority: Priority.HIGH,
};

describe('InvariantCard', () => {
  const mockOnUpdate = jest.fn();

  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('renders invariant information correctly', () => {
    render(<InvariantCard invariant={mockInvariant} onUpdate={mockOnUpdate} />);

    expect(screen.getByText('Test invariant description')).toBeInTheDocument();
    expect(screen.getByText('∀x ∈ ℝ, x² ≥ 0')).toBeInTheDocument();
    expect(screen.getByText('All real numbers squared are non-negative')).toBeInTheDocument();
    expect(screen.getByText('EXTRACTED')).toBeInTheDocument();
    expect(screen.getByText('HIGH')).toBeInTheDocument();
    expect(screen.getByText('High Confidence')).toBeInTheDocument();
  });

  it('displays variables correctly', () => {
    render(<InvariantCard invariant={mockInvariant} onUpdate={mockOnUpdate} />);

    expect(screen.getByText('x')).toBeInTheDocument();
    expect(screen.getByText('real')).toBeInTheDocument();
  });

  it('displays tags correctly', () => {
    render(<InvariantCard invariant={mockInvariant} onUpdate={mockOnUpdate} />);

    expect(screen.getByText('mathematics')).toBeInTheDocument();
    expect(screen.getByText('algebra')).toBeInTheDocument();
  });

  it('shows action buttons for EXTRACTED status', () => {
    render(<InvariantCard invariant={mockInvariant} onUpdate={mockOnUpdate} />);

    expect(screen.getByText('Rename')).toBeInTheDocument();
    expect(screen.getByText('Edit')).toBeInTheDocument();
    expect(screen.getByText('Split')).toBeInTheDocument();
    expect(screen.getByText('Confirm')).toBeInTheDocument();
    expect(screen.getByText('Reject')).toBeInTheDocument();
  });

  it('shows correct buttons for CONFIRMED status', () => {
    const confirmedInvariant = { ...mockInvariant, status: InvariantStatus.CONFIRMED };
    render(<InvariantCard invariant={confirmedInvariant} onUpdate={mockOnUpdate} />);

    expect(screen.queryByText('Rename')).not.toBeInTheDocument();
    expect(screen.queryByText('Edit')).not.toBeInTheDocument();
    expect(screen.queryByText('Split')).not.toBeInTheDocument();
    expect(screen.queryByText('Confirm')).not.toBeInTheDocument();
    expect(screen.getByText('Reject')).toBeInTheDocument();
  });

  it('shows correct buttons for REJECTED status', () => {
    const rejectedInvariant = { ...mockInvariant, status: InvariantStatus.REJECTED };
    render(<InvariantCard invariant={rejectedInvariant} onUpdate={mockOnUpdate} />);

    expect(screen.queryByText('Rename')).not.toBeInTheDocument();
    expect(screen.queryByText('Edit')).not.toBeInTheDocument();
    expect(screen.queryByText('Split')).not.toBeInTheDocument();
    expect(screen.queryByText('Reject')).not.toBeInTheDocument();
    expect(screen.getByText('Restore')).toBeInTheDocument();
  });

  it('displays confidence score correctly', () => {
    render(<InvariantCard invariant={mockInvariant} onUpdate={mockOnUpdate} />);

    expect(screen.getByText('Confidence: 95%')).toBeInTheDocument();
  });

  it('displays extraction date correctly', () => {
    render(<InvariantCard invariant={mockInvariant} onUpdate={mockOnUpdate} />);

    expect(screen.getByText(/Extracted: 1\/1\/2024/)).toBeInTheDocument();
  });

  it('applies correct status badge styling', () => {
    render(<InvariantCard invariant={mockInvariant} onUpdate={mockOnUpdate} />);

    const statusBadge = screen.getByText('EXTRACTED');
    expect(statusBadge).toHaveClass('status-badge', 'extracted');
  });

  it('applies correct priority badge styling', () => {
    render(<InvariantCard invariant={mockInvariant} onUpdate={mockOnUpdate} />);

    const priorityBadge = screen.getByText('HIGH');
    expect(priorityBadge).toHaveClass('priority-badge', 'high');
  });
}); 