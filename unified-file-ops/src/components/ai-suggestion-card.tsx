/**
 * AI Suggestion Card (T-043)
 *
 * Displays an AI-generated suggestion with Accept/Dismiss actions.
 * Shows category, confidence, description, and action payload preview.
 */
import { useState } from "react";
import { cn } from "@ufop/ui-components";
import type { AiSuggestion } from "@/stores/ai-store";
import {
  RefreshCw,
  Filter,
  Trash2,
  FolderTree,
  Copy,
  Zap,
  Shield,
  Check,
  X,
  ChevronDown,
  ChevronUp,
} from "lucide-react";

// ──────────────────────────────────────────────
// Category configuration
// ──────────────────────────────────────────────

const categoryConfig: Record<
  string,
  { icon: React.ReactNode; color: string; label: string }
> = {
  sync_rule: {
    icon: <RefreshCw className="h-3.5 w-3.5" />,
    color: "text-blue-500",
    label: "Sync Rule",
  },
  exclusion: {
    icon: <Filter className="h-3.5 w-3.5" />,
    color: "text-purple-500",
    label: "Exclusion",
  },
  cleanup: {
    icon: <Trash2 className="h-3.5 w-3.5" />,
    color: "text-orange-500",
    label: "Cleanup",
  },
  folder_classification: {
    icon: <FolderTree className="h-3.5 w-3.5" />,
    color: "text-green-500",
    label: "Folder Classification",
  },
  duplicate_risk: {
    icon: <Copy className="h-3.5 w-3.5" />,
    color: "text-amber-500",
    label: "Duplicate Risk",
  },
  performance: {
    icon: <Zap className="h-3.5 w-3.5" />,
    color: "text-cyan-500",
    label: "Performance",
  },
  security: {
    icon: <Shield className="h-3.5 w-3.5" />,
    color: "text-red-500",
    label: "Security",
  },
};

const defaultCategory = {
  icon: <Zap className="h-3.5 w-3.5" />,
  color: "text-[color:var(--color-primary)]",
  label: "Suggestion",
};

// ──────────────────────────────────────────────
// AiSuggestionCard Component
// ──────────────────────────────────────────────

interface AiSuggestionCardProps {
  suggestion: AiSuggestion;
  onAccept: () => void;
  onDismiss: () => void;
}

export function AiSuggestionCard({
  suggestion,
  onAccept,
  onDismiss,
}: AiSuggestionCardProps) {
  const [expanded, setExpanded] = useState(false);
  const [actionState, setActionState] = useState<"idle" | "accepting" | "dismissing">("idle");

  const config = categoryConfig[suggestion.category] || defaultCategory;

  const handleAccept = async () => {
    setActionState("accepting");
    try {
      await onAccept();
    } finally {
      setActionState("idle");
    }
  };

  const handleDismiss = async () => {
    setActionState("dismissing");
    try {
      await onDismiss();
    } finally {
      setActionState("idle");
    }
  };

  // Parse action payload for preview
  let actionPreview: Record<string, unknown> | null = null;
  if (suggestion.action_payload) {
    try {
      actionPreview = JSON.parse(suggestion.action_payload);
    } catch {
      // Ignore parse errors
    }
  }

  return (
    <div
      className={cn(
        "rounded-[var(--radius-md)] border border-[var(--color-border)]",
        "bg-[var(--color-bg)] overflow-hidden",
        "transition-all duration-200",
        "hover:border-[var(--color-primary)] hover:shadow-sm",
      )}
      data-testid={`suggestion-card-${suggestion.id}`}
      role="article"
      aria-label={`AI suggestion: ${suggestion.title}`}
    >
      {/* Header */}
      <div className="px-3 py-2">
        <div className="flex items-start gap-2">
          {/* Category icon */}
          <span className={cn("shrink-0 mt-0.5", config.color)}>
            {config.icon}
          </span>

          <div className="flex-1 min-w-0">
            {/* Category label + confidence */}
            <div className="flex items-center gap-2 mb-0.5">
              <span
                className={cn(
                  "text-[10px] font-medium uppercase tracking-wider",
                  config.color,
                )}
              >
                {config.label}
              </span>
              <ConfidenceBadge confidence={suggestion.confidence} />
            </div>

            {/* Title */}
            <h4 className="text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text)] leading-snug">
              {suggestion.title}
            </h4>

            {/* Description */}
            <p className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)] mt-1 leading-relaxed">
              {suggestion.description}
            </p>
          </div>
        </div>

        {/* Action preview toggle */}
        {actionPreview && (
          <button
            className={cn(
              "flex items-center gap-1 mt-2 text-[length:var(--font-size-xs)]",
              "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]",
              "transition-theme",
            )}
            onClick={() => setExpanded(!expanded)}
            aria-expanded={expanded}
          >
            {expanded ? (
              <ChevronUp className="h-3 w-3" />
            ) : (
              <ChevronDown className="h-3 w-3" />
            )}
            {expanded ? "Hide details" : "Show details"}
          </button>
        )}

        {/* Expanded action payload preview */}
        {expanded && actionPreview && (
          <div className="mt-2 p-2 bg-[var(--color-bg-secondary)] rounded-[var(--radius-sm)]">
            <pre className="text-[10px] text-[color:var(--color-text-secondary)] overflow-x-auto whitespace-pre-wrap">
              {JSON.stringify(actionPreview, null, 2)}
            </pre>
          </div>
        )}
      </div>

      {/* Action buttons */}
      <div className="flex border-t border-[var(--color-border)]">
        <button
          className={cn(
            "flex-1 flex items-center justify-center gap-1.5 px-3 py-1.5",
            "text-[length:var(--font-size-xs)] font-medium",
            "text-[color:var(--color-primary)] hover:bg-[var(--color-primary)] hover:text-[color:var(--color-primary-foreground)]",
            "transition-theme",
            "disabled:opacity-50 disabled:cursor-not-allowed",
          )}
          onClick={handleAccept}
          disabled={actionState !== "idle"}
          aria-label={`Accept suggestion: ${suggestion.title}`}
          data-testid={`accept-suggestion-${suggestion.id}`}
        >
          {actionState === "accepting" ? (
            <span className="h-3 w-3 border border-current border-t-transparent rounded-full animate-spin" />
          ) : (
            <Check className="h-3.5 w-3.5" />
          )}
          Accept
        </button>

        <div className="w-px bg-[var(--color-border)]" />

        <button
          className={cn(
            "flex-1 flex items-center justify-center gap-1.5 px-3 py-1.5",
            "text-[length:var(--font-size-xs)]",
            "text-[color:var(--color-text-secondary)] hover:bg-[var(--color-hover-bg)] hover:text-[color:var(--color-text)]",
            "transition-theme",
            "disabled:opacity-50 disabled:cursor-not-allowed",
          )}
          onClick={handleDismiss}
          disabled={actionState !== "idle"}
          aria-label={`Dismiss suggestion: ${suggestion.title}`}
          data-testid={`dismiss-suggestion-${suggestion.id}`}
        >
          {actionState === "dismissing" ? (
            <span className="h-3 w-3 border border-current border-t-transparent rounded-full animate-spin" />
          ) : (
            <X className="h-3.5 w-3.5" />
          )}
          Dismiss
        </button>
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Confidence Badge
// ──────────────────────────────────────────────

function ConfidenceBadge({ confidence }: { confidence: number }) {
  const percent = Math.round(confidence * 100);

  const colorClass =
    confidence > 0.7
      ? "bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400"
      : confidence > 0.4
        ? "bg-amber-100 text-amber-700 dark:bg-amber-900/30 dark:text-amber-400"
        : "bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400";

  return (
    <span
      className={cn(
        "px-1.5 py-0.5 rounded-full text-[10px] font-medium",
        colorClass,
      )}
      title={`${percent}% confidence`}
    >
      {percent}%
    </span>
  );
}
