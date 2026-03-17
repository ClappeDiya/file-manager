/**
 * AI Panel (T-043, T-044, T-045)
 *
 * Sidebar panel for AI assistant features:
 * - Chat interface for help and error explanations
 * - Natural language job creation input
 * - Suggestion cards with Accept/Dismiss
 * - AI audit log viewer
 * - Enterprise feature toggles
 */
import { useState, useCallback, useRef, useEffect } from "react";
import { useAiStore, type AiChatMessage, type ParsedJobConfig } from "@/stores/ai-store";
import { cn } from "@ufop/ui-components";
import {
  Bot,
  Send,
  X,
  Lightbulb,
  Shield,
  Trash2,
  ChevronDown,
  ChevronUp,
  Sparkles,
  AlertTriangle,
  CheckCircle,
  MessageSquare,
  ScrollText,
  Settings,
  ToggleLeft,
  ToggleRight,
} from "lucide-react";
import { AiSuggestionCard } from "./ai-suggestion-card";

// ──────────────────────────────────────────────
// AI Panel - Main Component
// ──────────────────────────────────────────────

interface AiPanelProps {
  className?: string;
}

export function AiPanel({ className }: AiPanelProps) {
  const panelOpen = useAiStore((s) => s.panelOpen);
  const activeTab = useAiStore((s) => s.activeTab);
  const setActiveTab = useAiStore((s) => s.setActiveTab);
  const setPanelOpen = useAiStore((s) => s.setPanelOpen);
  const featureToggles = useAiStore((s) => s.featureToggles);

  if (!panelOpen) return null;

  // If AI is completely disabled, show a minimal panel
  if (!featureToggles.ai_master_enabled) {
    return (
      <div
        className={cn(
          "flex flex-col border-l border-[var(--color-border)]",
          "bg-[var(--color-bg-elevated)] w-80",
          className,
        )}
        data-testid="ai-panel"
        role="complementary"
        aria-label="AI Assistant (Disabled)"
      >
        <AiPanelHeader onClose={() => setPanelOpen(false)} />
        <div className="flex-1 flex items-center justify-center p-6 text-center">
          <div>
            <Bot className="h-12 w-12 mx-auto mb-3 text-[color:var(--color-text-tertiary)]" />
            <p className="text-[length:var(--font-size-sm)] text-[color:var(--color-text-secondary)]">
              AI features are disabled
            </p>
            <p className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)] mt-1">
              Enable them in Settings to get AI-powered help
            </p>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div
      className={cn(
        "flex flex-col border-l border-[var(--color-border)]",
        "bg-[var(--color-bg-elevated)] w-80",
        className,
      )}
      data-testid="ai-panel"
      role="complementary"
      aria-label="AI Assistant"
    >
      <AiPanelHeader onClose={() => setPanelOpen(false)} />

      {/* Tab bar */}
      <div className="flex border-b border-[var(--color-border)]">
        <TabButton
          active={activeTab === "chat"}
          onClick={() => setActiveTab("chat")}
          icon={<MessageSquare className="h-3.5 w-3.5" />}
          label="Chat"
        />
        <TabButton
          active={activeTab === "suggestions"}
          onClick={() => setActiveTab("suggestions")}
          icon={<Lightbulb className="h-3.5 w-3.5" />}
          label="Suggestions"
        />
        <TabButton
          active={activeTab === "audit"}
          onClick={() => setActiveTab("audit")}
          icon={<ScrollText className="h-3.5 w-3.5" />}
          label="Audit"
        />
      </div>

      {/* Tab content */}
      <div className="flex-1 overflow-hidden">
        {activeTab === "chat" && <ChatTab />}
        {activeTab === "suggestions" && <SuggestionsTab />}
        {activeTab === "audit" && <AuditTab />}
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Panel Header
// ──────────────────────────────────────────────

function AiPanelHeader({ onClose }: { onClose: () => void }) {
  return (
    <div className="flex items-center justify-between px-3 py-2 border-b border-[var(--color-border)]">
      <div className="flex items-center gap-2">
        <Bot className="h-4 w-4 text-[color:var(--color-primary)]" />
        <span className="text-[length:var(--font-size-sm)] font-semibold text-[color:var(--color-text)]">
          AI Assistant
        </span>
      </div>
      <button
        className={cn(
          "h-6 w-6 flex items-center justify-center rounded-sm",
          "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]",
          "hover:bg-[var(--color-hover-bg)] transition-theme",
        )}
        onClick={onClose}
        aria-label="Close AI panel"
      >
        <X className="h-4 w-4" />
      </button>
    </div>
  );
}

// ──────────────────────────────────────────────
// Tab Button
// ──────────────────────────────────────────────

function TabButton({
  active,
  onClick,
  icon,
  label,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  label: string;
}) {
  return (
    <button
      className={cn(
        "flex-1 flex items-center justify-center gap-1.5 px-2 py-2",
        "text-[length:var(--font-size-xs)] transition-theme",
        active
          ? "text-[color:var(--color-primary)] border-b-2 border-[var(--color-primary)] font-medium"
          : "text-[color:var(--color-text-secondary)] hover:text-[color:var(--color-text)] hover:bg-[var(--color-hover-bg)]",
      )}
      onClick={onClick}
      aria-selected={active}
      role="tab"
    >
      {icon}
      {label}
    </button>
  );
}

// ──────────────────────────────────────────────
// Chat Tab (T-043, T-044)
// ──────────────────────────────────────────────

function ChatTab() {
  const chatMessages = useAiStore((s) => s.chatMessages);
  const chatLoading = useAiStore((s) => s.chatLoading);
  const sendMessage = useAiStore((s) => s.sendMessage);
  const clearChat = useAiStore((s) => s.clearChat);
  const parseNaturalLanguage = useAiStore((s) => s.parseNaturalLanguage);
  const currentParsedJob = useAiStore((s) => s.currentParsedJob);
  const clearParsedJob = useAiStore((s) => s.clearParsedJob);
  const currentExplanation = useAiStore((s) => s.currentExplanation);

  const [input, setInput] = useState("");
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  // Auto-scroll to bottom on new messages
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [chatMessages]);

  const handleSend = useCallback(async () => {
    const message = input.trim();
    if (!message) return;
    setInput("");

    // Check if this looks like a natural language job command
    const nlKeywords = ["sync", "backup", "transfer", "send", "upload", "exclude", "rename"];
    const isNlCommand = nlKeywords.some((kw) => message.toLowerCase().includes(kw)) &&
      !message.toLowerCase().startsWith("how") &&
      !message.toLowerCase().startsWith("what") &&
      !message.toLowerCase().startsWith("why") &&
      !message.toLowerCase().startsWith("help");

    if (isNlCommand) {
      try {
        await parseNaturalLanguage(message);
      } catch {
        // Falls back to regular chat
        await sendMessage(message);
      }
    } else {
      await sendMessage(message);
    }
  }, [input, sendMessage, parseNaturalLanguage]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend],
  );

  return (
    <div className="flex flex-col h-full">
      {/* Error Explanation Banner */}
      {currentExplanation && (
        <ErrorExplanationBanner explanation={currentExplanation} />
      )}

      {/* NL Job Preview */}
      {currentParsedJob && (
        <NlJobPreview config={currentParsedJob} onDismiss={clearParsedJob} />
      )}

      {/* Messages */}
      <div className="flex-1 overflow-y-auto px-3 py-2 space-y-3">
        {chatMessages.length === 0 && (
          <div className="text-center py-8">
            <Sparkles className="h-8 w-8 mx-auto mb-2 text-[color:var(--color-primary)] opacity-50" />
            <p className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
              Ask me anything about file operations, or describe a sync job in plain language.
            </p>
          </div>
        )}
        {chatMessages.map((msg) => (
          <ChatBubble key={msg.id} message={msg} />
        ))}
        {chatLoading && (
          <div className="flex items-center gap-2 px-3 py-2">
            <span className="h-2 w-2 rounded-full bg-[var(--color-primary)] animate-pulse" />
            <span className="h-2 w-2 rounded-full bg-[var(--color-primary)] animate-pulse delay-100" />
            <span className="h-2 w-2 rounded-full bg-[var(--color-primary)] animate-pulse delay-200" />
          </div>
        )}
        <div ref={messagesEndRef} />
      </div>

      {/* Input area */}
      <div className="border-t border-[var(--color-border)] p-2">
        <div className="flex items-end gap-2">
          <textarea
            ref={inputRef}
            className={cn(
              "flex-1 resize-none rounded-[var(--radius-md)]",
              "bg-[var(--color-bg)] border border-[var(--color-border)]",
              "px-3 py-2 text-[length:var(--font-size-xs)]",
              "text-[color:var(--color-text)] placeholder:text-[color:var(--color-text-tertiary)]",
              "focus:outline-none focus:ring-2 focus:ring-[var(--color-focus-ring)]",
            )}
            placeholder="Ask for help or describe a task..."
            rows={2}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            aria-label="AI chat input"
            data-testid="ai-chat-input"
          />
          <div className="flex flex-col gap-1">
            <button
              className={cn(
                "h-8 w-8 flex items-center justify-center rounded-[var(--radius-md)]",
                "bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)]",
                "hover:opacity-90 transition-theme",
                "disabled:opacity-50 disabled:cursor-not-allowed",
              )}
              onClick={handleSend}
              disabled={!input.trim() || chatLoading}
              aria-label="Send message"
              data-testid="ai-chat-send"
            >
              <Send className="h-3.5 w-3.5" />
            </button>
            {chatMessages.length > 0 && (
              <button
                className={cn(
                  "h-6 w-6 flex items-center justify-center rounded-sm",
                  "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-error)]",
                  "hover:bg-[var(--color-hover-bg)] transition-theme",
                )}
                onClick={clearChat}
                aria-label="Clear chat"
                title="Clear chat history"
              >
                <Trash2 className="h-3 w-3" />
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Chat Bubble
// ──────────────────────────────────────────────

function ChatBubble({ message }: { message: AiChatMessage }) {
  const isUser = message.role === "user";

  return (
    <div
      className={cn("flex", isUser ? "justify-end" : "justify-start")}
      data-testid={`chat-message-${message.role}`}
    >
      <div
        className={cn(
          "max-w-[85%] rounded-lg px-3 py-2",
          "text-[length:var(--font-size-xs)] leading-relaxed",
          isUser
            ? "bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)]"
            : "bg-[var(--color-bg-secondary)] text-[color:var(--color-text)]",
        )}
      >
        <div className="whitespace-pre-wrap">{message.content}</div>
        <div
          className={cn(
            "text-[10px] mt-1 opacity-60",
            isUser ? "text-right" : "text-left",
          )}
        >
          {new Date(message.timestamp).toLocaleTimeString([], {
            hour: "2-digit",
            minute: "2-digit",
          })}
        </div>
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Error Explanation Banner (T-043)
// ──────────────────────────────────────────────

function ErrorExplanationBanner({
  explanation,
}: {
  explanation: {
    plain_language: string;
    possible_causes: string[];
    suggested_fixes: string[];
  };
}) {
  const [expanded, setExpanded] = useState(true);

  return (
    <div
      className="border-b border-[var(--color-border)] bg-[var(--color-warning-bg)] p-3"
      data-testid="error-explanation"
    >
      <button
        className="flex items-center gap-2 w-full text-left"
        onClick={() => setExpanded(!expanded)}
      >
        <AlertTriangle className="h-4 w-4 text-[color:var(--color-warning)] shrink-0" />
        <span className="text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text)] flex-1">
          Error Explanation
        </span>
        {expanded ? (
          <ChevronUp className="h-3.5 w-3.5 text-[color:var(--color-text-tertiary)]" />
        ) : (
          <ChevronDown className="h-3.5 w-3.5 text-[color:var(--color-text-tertiary)]" />
        )}
      </button>

      {expanded && (
        <div className="mt-2 space-y-2">
          <p className="text-[length:var(--font-size-xs)] text-[color:var(--color-text)]">
            {explanation.plain_language}
          </p>

          {explanation.possible_causes.length > 0 && (
            <div>
              <p className="text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text-secondary)] mb-1">
                Possible causes:
              </p>
              <ul className="list-disc list-inside space-y-0.5">
                {explanation.possible_causes.map((cause, i) => (
                  <li
                    key={i}
                    className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)]"
                  >
                    {cause}
                  </li>
                ))}
              </ul>
            </div>
          )}

          {explanation.suggested_fixes.length > 0 && (
            <div>
              <p className="text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text-secondary)] mb-1">
                Suggested fixes:
              </p>
              <ul className="space-y-1">
                {explanation.suggested_fixes.map((fix, i) => (
                  <li
                    key={i}
                    className="flex items-start gap-1.5 text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)]"
                  >
                    <CheckCircle className="h-3 w-3 text-[color:var(--color-success)] shrink-0 mt-0.5" />
                    {fix}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────
// NL Job Preview (T-044)
// ──────────────────────────────────────────────

function NlJobPreview({
  config,
  onDismiss,
}: {
  config: ParsedJobConfig;
  onDismiss: () => void;
}) {
  const [showAdvanced, setShowAdvanced] = useState(false);

  return (
    <div
      className="border-b border-[var(--color-border)] bg-[var(--color-primary-bg)] p-3"
      data-testid="nl-job-preview"
    >
      <div className="flex items-center justify-between mb-2">
        <div className="flex items-center gap-2">
          <Sparkles className="h-4 w-4 text-[color:var(--color-primary)]" />
          <span className="text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text)]">
            Parsed Job Configuration
          </span>
        </div>
        <button
          className="text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]"
          onClick={onDismiss}
          aria-label="Dismiss job preview"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>

      {/* Confidence indicator */}
      <div className="flex items-center gap-2 mb-2">
        <div className="flex-1 h-1.5 bg-[var(--color-border)] rounded-full overflow-hidden">
          <div
            className={cn(
              "h-full rounded-full",
              config.confidence > 0.7
                ? "bg-[var(--color-success)]"
                : config.confidence > 0.4
                  ? "bg-[var(--color-warning)]"
                  : "bg-[var(--color-error)]",
            )}
            style={{ width: `${config.confidence * 100}%` }}
          />
        </div>
        <span className="text-[10px] text-[color:var(--color-text-tertiary)]">
          {Math.round(config.confidence * 100)}% confident
        </span>
      </div>

      {/* Clarification questions */}
      {config.needs_clarification && config.clarification_questions.length > 0 && (
        <div className="mb-2 p-2 bg-[var(--color-warning-bg)] rounded-[var(--radius-sm)]">
          <p className="text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-warning)] mb-1">
            Need more information:
          </p>
          <ul className="space-y-1">
            {config.clarification_questions.map((q, i) => (
              <li
                key={i}
                className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)]"
              >
                - {q}
              </li>
            ))}
          </ul>
        </div>
      )}

      {/* Summary */}
      <div className="space-y-1 text-[length:var(--font-size-xs)]">
        <div className="flex justify-between">
          <span className="text-[color:var(--color-text-secondary)]">Type:</span>
          <span className="text-[color:var(--color-text)] font-medium capitalize">{config.intent}</span>
        </div>
        {config.source_path && (
          <div className="flex justify-between">
            <span className="text-[color:var(--color-text-secondary)]">Source:</span>
            <span className="text-[color:var(--color-text)] truncate ml-2">{config.source_path}</span>
          </div>
        )}
        {config.dest_connector && (
          <div className="flex justify-between">
            <span className="text-[color:var(--color-text-secondary)]">Destination:</span>
            <span className="text-[color:var(--color-text)] capitalize">{config.dest_connector.replace(/_/g, " ")}</span>
          </div>
        )}
        {config.schedule_human && (
          <div className="flex justify-between">
            <span className="text-[color:var(--color-text-secondary)]">Schedule:</span>
            <span className="text-[color:var(--color-text)]">{config.schedule_human}</span>
          </div>
        )}
      </div>

      {/* Advanced details toggle */}
      {config.preview_config && (
        <button
          className="flex items-center gap-1 mt-2 text-[length:var(--font-size-xs)] text-[color:var(--color-primary)]"
          onClick={() => setShowAdvanced(!showAdvanced)}
        >
          <Settings className="h-3 w-3" />
          {showAdvanced ? "Hide" : "Show"} advanced config
        </button>
      )}

      {showAdvanced && config.preview_config && (
        <pre className="mt-2 p-2 bg-[var(--color-bg)] rounded-[var(--radius-sm)] text-[10px] text-[color:var(--color-text-secondary)] overflow-x-auto">
          {JSON.stringify(JSON.parse(config.preview_config), null, 2)}
        </pre>
      )}

      {/* Action buttons */}
      {!config.needs_clarification && (
        <div className="flex gap-2 mt-3">
          <button
            className={cn(
              "flex-1 px-3 py-1.5 rounded-[var(--radius-md)]",
              "bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)]",
              "text-[length:var(--font-size-xs)] font-medium",
              "hover:opacity-90 transition-theme",
            )}
            data-testid="nl-job-activate"
          >
            Activate
          </button>
          <button
            className={cn(
              "px-3 py-1.5 rounded-[var(--radius-md)]",
              "border border-[var(--color-border)]",
              "text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)]",
              "hover:bg-[var(--color-hover-bg)] transition-theme",
            )}
            onClick={onDismiss}
          >
            Cancel
          </button>
        </div>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────
// Suggestions Tab (T-043)
// ──────────────────────────────────────────────

function SuggestionsTab() {
  const suggestions = useAiStore((s) => s.suggestions);
  const suggestionsLoading = useAiStore((s) => s.suggestionsLoading);
  const dismissSuggestion = useAiStore((s) => s.dismissSuggestion);
  const acceptSuggestion = useAiStore((s) => s.acceptSuggestion);
  const generateSuggestions = useAiStore((s) => s.generateSuggestions);
  const confirmationPending = useAiStore((s) => s.confirmationPending);
  const confirmAction = useAiStore((s) => s.confirmAction);

  const handleRefresh = useCallback(() => {
    generateSuggestions({
      current_path: null,
      total_files: 0,
      temp_file_count: 0,
      duplicate_candidates: 0,
      unclassified_folders: 0,
      recent_errors: [],
      active_sync_pairs: 0,
    });
  }, [generateSuggestions]);

  return (
    <div className="flex flex-col h-full">
      {/* Confirmation dialog */}
      {confirmationPending && (
        <ConfirmationBanner
          actionId={confirmationPending}
          onConfirm={confirmAction}
          onCancel={() => useAiStore.setState({ confirmationPending: null })}
        />
      )}

      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        {suggestionsLoading && (
          <div className="flex items-center justify-center py-8">
            <span className="h-4 w-4 border-2 border-[var(--color-primary)] border-t-transparent rounded-full animate-spin" />
          </div>
        )}

        {!suggestionsLoading && suggestions.length === 0 && (
          <div className="text-center py-8">
            <Lightbulb className="h-8 w-8 mx-auto mb-2 text-[color:var(--color-text-tertiary)] opacity-50" />
            <p className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
              No suggestions right now
            </p>
            <button
              className={cn(
                "mt-3 px-3 py-1.5 rounded-[var(--radius-md)]",
                "text-[length:var(--font-size-xs)] text-[color:var(--color-primary)]",
                "border border-[var(--color-primary)]",
                "hover:bg-[var(--color-primary)] hover:text-[color:var(--color-primary-foreground)]",
                "transition-theme",
              )}
              onClick={handleRefresh}
            >
              Analyze workspace
            </button>
          </div>
        )}

        {suggestions.map((suggestion) => (
          <AiSuggestionCard
            key={suggestion.id}
            suggestion={suggestion}
            onAccept={() => acceptSuggestion(suggestion.id)}
            onDismiss={() => dismissSuggestion(suggestion.id)}
          />
        ))}
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Confirmation Banner (T-045)
// ──────────────────────────────────────────────

function ConfirmationBanner({
  actionId,
  onConfirm,
  onCancel,
}: {
  actionId: string;
  onConfirm: (id: string) => Promise<boolean>;
  onCancel: () => void;
}) {
  return (
    <div
      className="border-b border-[var(--color-border)] bg-[var(--color-error-bg)] p-3"
      data-testid="confirmation-banner"
    >
      <div className="flex items-start gap-2">
        <Shield className="h-4 w-4 text-[color:var(--color-error)] shrink-0 mt-0.5" />
        <div className="flex-1">
          <p className="text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-error)]">
            This action may modify or delete files
          </p>
          <p className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)] mt-1">
            Please confirm you want to proceed with this destructive action.
          </p>
          <div className="flex gap-2 mt-2">
            <button
              className={cn(
                "px-3 py-1 rounded-[var(--radius-md)]",
                "bg-[var(--color-error)] text-white",
                "text-[length:var(--font-size-xs)] font-medium",
                "hover:opacity-90 transition-theme",
              )}
              onClick={() => onConfirm(actionId)}
              data-testid="confirm-destructive-action"
            >
              Confirm
            </button>
            <button
              className={cn(
                "px-3 py-1 rounded-[var(--radius-md)]",
                "border border-[var(--color-border)]",
                "text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)]",
                "hover:bg-[var(--color-hover-bg)] transition-theme",
              )}
              onClick={onCancel}
            >
              Cancel
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Audit Tab (T-045)
// ──────────────────────────────────────────────

function AuditTab() {
  const auditLog = useAiStore((s) => s.auditLog);
  const loadAuditLog = useAiStore((s) => s.loadAuditLog);
  const featureToggles = useAiStore((s) => s.featureToggles);
  const setFeatureToggles = useAiStore((s) => s.setFeatureToggles);
  const [showToggles, setShowToggles] = useState(false);

  useEffect(() => {
    loadAuditLog();
  }, [loadAuditLog]);

  return (
    <div className="flex flex-col h-full">
      {/* Feature toggles section */}
      <div className="border-b border-[var(--color-border)]">
        <button
          className={cn(
            "flex items-center justify-between w-full px-3 py-2",
            "text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text-secondary)]",
            "hover:bg-[var(--color-hover-bg)] transition-theme",
          )}
          onClick={() => setShowToggles(!showToggles)}
        >
          <div className="flex items-center gap-2">
            <Settings className="h-3.5 w-3.5" />
            AI Feature Controls
          </div>
          {showToggles ? (
            <ChevronUp className="h-3.5 w-3.5" />
          ) : (
            <ChevronDown className="h-3.5 w-3.5" />
          )}
        </button>

        {showToggles && (
          <div className="px-3 pb-3 space-y-2">
            <ToggleRow
              label="AI Master Toggle"
              enabled={featureToggles.ai_master_enabled}
              onChange={(v) =>
                setFeatureToggles({ ...featureToggles, ai_master_enabled: v })
              }
            />
            <ToggleRow
              label="Error Explanations"
              enabled={featureToggles.error_explanations}
              onChange={(v) =>
                setFeatureToggles({ ...featureToggles, error_explanations: v })
              }
              disabled={!featureToggles.ai_master_enabled}
            />
            <ToggleRow
              label="Suggestions"
              enabled={featureToggles.suggestions}
              onChange={(v) =>
                setFeatureToggles({ ...featureToggles, suggestions: v })
              }
              disabled={!featureToggles.ai_master_enabled}
            />
            <ToggleRow
              label="NL Job Creation"
              enabled={featureToggles.nl_job_creation}
              onChange={(v) =>
                setFeatureToggles({ ...featureToggles, nl_job_creation: v })
              }
              disabled={!featureToggles.ai_master_enabled}
            />
            <ToggleRow
              label="Content Analysis (opt-in)"
              enabled={featureToggles.content_analysis_opt_in}
              onChange={(v) =>
                setFeatureToggles({ ...featureToggles, content_analysis_opt_in: v })
              }
              disabled={!featureToggles.ai_master_enabled}
            />
          </div>
        )}
      </div>

      {/* Audit log entries */}
      <div className="flex-1 overflow-y-auto">
        {auditLog.length === 0 ? (
          <div className="text-center py-8">
            <ScrollText className="h-8 w-8 mx-auto mb-2 text-[color:var(--color-text-tertiary)] opacity-50" />
            <p className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
              No AI actions recorded yet
            </p>
          </div>
        ) : (
          <div className="divide-y divide-[var(--color-border)]">
            {auditLog.map((entry) => (
              <div key={entry.id} className="px-3 py-2" data-testid="audit-entry">
                <div className="flex items-center gap-2 mb-1">
                  {entry.destructive ? (
                    <AlertTriangle className="h-3 w-3 text-[color:var(--color-error)]" />
                  ) : (
                    <CheckCircle className="h-3 w-3 text-[color:var(--color-success)]" />
                  )}
                  <span className="text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text)]">
                    {entry.action_type.replace(/_/g, " ")}
                  </span>
                  <span className="text-[10px] text-[color:var(--color-text-tertiary)] ml-auto">
                    {new Date(entry.timestamp).toLocaleTimeString([], {
                      hour: "2-digit",
                      minute: "2-digit",
                    })}
                  </span>
                </div>
                <p className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)] truncate">
                  {entry.input_summary}
                </p>
                {entry.user_confirmed && (
                  <span className="text-[10px] text-[color:var(--color-success)]">User confirmed</span>
                )}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Toggle Row (T-045 Enterprise Controls)
// ──────────────────────────────────────────────

function ToggleRow({
  label,
  enabled,
  onChange,
  disabled = false,
}: {
  label: string;
  enabled: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <div
      className={cn(
        "flex items-center justify-between py-1",
        disabled && "opacity-50",
      )}
    >
      <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text)]">{label}</span>
      <button
        className="text-[color:var(--color-primary)]"
        onClick={() => !disabled && onChange(!enabled)}
        disabled={disabled}
        aria-label={`${label}: ${enabled ? "enabled" : "disabled"}`}
        aria-pressed={enabled}
        data-testid={`toggle-${label.toLowerCase().replace(/\s+/g, "-")}`}
      >
        {enabled ? (
          <ToggleRight className="h-5 w-5" />
        ) : (
          <ToggleLeft className="h-5 w-5 text-[color:var(--color-text-tertiary)]" />
        )}
      </button>
    </div>
  );
}
