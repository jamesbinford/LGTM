-- Initial schema for AI Code Review Pipeline
-- Phase 2: PostgreSQL persistence

-- Reviews table: top-level record for each PR review
CREATE TABLE reviews (
    id UUID PRIMARY KEY,
    pr_number BIGINT NOT NULL,
    repo VARCHAR(255) NOT NULL,
    branch VARCHAR(255),
    commit_sha VARCHAR(40) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    status VARCHAR(20) NOT NULL DEFAULT 'pending',

    -- Ensure one active review per PR
    CONSTRAINT valid_status CHECK (status IN ('pending', 'decided', 'applied', 'stale'))
);

-- Index for looking up reviews by repo and PR
CREATE INDEX idx_reviews_repo_pr ON reviews(repo, pr_number);

-- Index for listing pending reviews
CREATE INDEX idx_reviews_pending ON reviews(status) WHERE status = 'pending';

-- Suggestions table: individual code review suggestions from Codex
CREATE TABLE suggestions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    review_id UUID NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
    external_id VARCHAR(50) NOT NULL,  -- S001, S002, etc.
    suggestion_type VARCHAR(50) NOT NULL,  -- security, performance, style, logic
    severity VARCHAR(20) NOT NULL,  -- critical, high, medium, low
    file_path VARCHAR(500) NOT NULL,
    line_start INTEGER NOT NULL,
    line_end INTEGER NOT NULL,
    description TEXT NOT NULL,
    proposed_fix TEXT,

    -- Claude's recommendation
    claude_action VARCHAR(20),  -- accept, reject, modify
    claude_confidence DOUBLE PRECISION,
    claude_rationale TEXT,
    claude_modified_fix TEXT,

    -- Human decision
    human_decision VARCHAR(20),  -- accepted, rejected, deferred
    human_reason TEXT,
    decided_by VARCHAR(100),
    decided_at TIMESTAMPTZ,

    CONSTRAINT valid_type CHECK (suggestion_type IN ('security', 'performance', 'style', 'logic', 'documentation')),
    CONSTRAINT valid_severity CHECK (severity IN ('critical', 'high', 'medium', 'low')),
    CONSTRAINT valid_claude_action CHECK (claude_action IS NULL OR claude_action IN ('accept', 'reject', 'modify')),
    CONSTRAINT valid_human_decision CHECK (human_decision IS NULL OR human_decision IN ('accepted', 'rejected', 'deferred')),

    -- Unique external_id per review
    CONSTRAINT unique_external_id_per_review UNIQUE (review_id, external_id)
);

-- Index for finding pending suggestions
CREATE INDEX idx_suggestions_pending ON suggestions(review_id) WHERE human_decision IS NULL;

-- Index for severity filtering
CREATE INDEX idx_suggestions_severity ON suggestions(severity);
