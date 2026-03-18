/**
 * AI Store (T-043..T-045)
 *
 * Zustand store for AI assistant state management.
 * Provides reactive state for chat, suggestions, NL job creation,
 * safety controls, and audit logging.
 */
import { create } from "zustand";
import { persist } from "zustand/middleware";
import { tauriInvoke } from "@/hooks/use-tauri";

// ── Types ──

export interface AiChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  timestamp: string;
}

export interface AiSuggestion {
  id: string;
  category: string;
  title: string;
  description: string;
  action_payload: string | null;
  confidence: number;
  created_at: string;
  dismissed: boolean;
}

export interface ErrorExplanation {
  original_error: string;
  plain_language: string;
  possible_causes: string[];
  suggested_fixes: string[];
  related_docs: string[];
}

export interface ParsedJobConfig {
  intent: string;
  source_path: string | null;
  dest_path: string | null;
  dest_connector: string | null;
  schedule: string | null;
  schedule_human: string | null;
  direction: string | null;
  filter_patterns: string[];
  exclude_patterns: string[];
  rename_pattern: string | null;
  confidence: number;
  needs_clarification: boolean;
  clarification_questions: string[];
  preview_config: string | null;
}

export interface AiAuditEntry {
  id: string;
  timestamp: string;
  action_type: string;
  input_summary: string;
  output_summary: string;
  user_confirmed: boolean;
  destructive: boolean;
  metadata: string | null;
}

export interface AiFeatureToggles {
  ai_master_enabled: boolean;
  error_explanations: boolean;
  suggestions: boolean;
  nl_job_creation: boolean;
  content_analysis_opt_in: boolean;
  model_routing: string;
}

// ── Store ──

interface AiState {
  // Chat
  chatMessages: AiChatMessage[];
  chatLoading: boolean;

  // Suggestions
  suggestions: AiSuggestion[];
  suggestionsLoading: boolean;

  // NL Job Creation
  currentParsedJob: ParsedJobConfig | null;
  nlLoading: boolean;

  // Error Explanation
  currentExplanation: ErrorExplanation | null;

  // Safety / Audit
  auditLog: AiAuditEntry[];
  featureToggles: AiFeatureToggles;
  confirmationPending: string | null;

  // UI state
  panelOpen: boolean;
  activeTab: "chat" | "suggestions" | "audit";
  error: string | null;

  // Actions
  sendMessage: (message: string) => Promise<AiChatMessage>;
  loadChatHistory: () => Promise<void>;
  clearChat: () => Promise<void>;
  explainError: (errorType: string, message: string, advice: string) => Promise<ErrorExplanation>;
  generateSuggestions: (context: Record<string, unknown>) => Promise<void>;
  loadSuggestions: () => Promise<void>;
  dismissSuggestion: (id: string) => Promise<void>;
  acceptSuggestion: (id: string) => Promise<string>;
  parseNaturalLanguage: (input: string) => Promise<ParsedJobConfig>;
  confirmAction: (actionId: string) => Promise<boolean>;
  loadAuditLog: (limit?: number, offset?: number) => Promise<void>;
  loadFeatureToggles: () => Promise<void>;
  setFeatureToggles: (toggles: AiFeatureToggles) => Promise<void>;

  // UI actions
  setPanelOpen: (open: boolean) => void;
  togglePanel: () => void;
  setActiveTab: (tab: "chat" | "suggestions" | "audit") => void;
  clearError: () => void;
  clearParsedJob: () => void;
}

export const useAiStore = create<AiState>()(
  persist(
    (set) => ({
      // Initial state
      chatMessages: [],
      chatLoading: false,
      suggestions: [],
      suggestionsLoading: false,
      currentParsedJob: null,
      nlLoading: false,
      currentExplanation: null,
      auditLog: [],
      featureToggles: {
        ai_master_enabled: true,
        error_explanations: true,
        suggestions: true,
        nl_job_creation: true,
        content_analysis_opt_in: false,
        model_routing: "local",
      },
      confirmationPending: null,
      panelOpen: false,
      activeTab: "chat",
      error: null,

      // Chat
      sendMessage: async (message: string) => {
        set({ chatLoading: true, error: null });

        // Optimistic: add user message immediately
        const userMsg: AiChatMessage = {
          id: `user-${Date.now()}`,
          role: "user",
          content: message,
          timestamp: new Date().toISOString(),
        };
        set((state) => ({
          chatMessages: [...state.chatMessages, userMsg],
        }));

        try {
          const response = await tauriInvoke<AiChatMessage>("ai_chat", { message });
          set((state) => ({
            chatMessages: [...state.chatMessages, response],
          }));
          return response;
        } catch (e: any) {
          // Fallback: structured error response
          const fallback: AiChatMessage = {
            id: `fallback-${Date.now()}`,
            role: "assistant",
            content: `I'm unable to process your request right now. Error: ${e?.message || "Unknown error"}. Please try again or check the error details panel.`,
            timestamp: new Date().toISOString(),
          };
          set((state) => ({
            chatMessages: [...state.chatMessages, fallback],
            error: e?.message || "Chat request failed",
          }));
          return fallback;
        } finally {
          set({ chatLoading: false });
        }
      },

      loadChatHistory: async () => {
        try {
          const messages = await tauriInvoke<AiChatMessage[]>("ai_get_chat_history", undefined, []);
          set({ chatMessages: messages });
        } catch (e: any) {
          set({ error: e?.message || "Failed to load chat history" });
        }
      },

      clearChat: async () => {
        try {
          await tauriInvoke("ai_clear_chat");
          set({ chatMessages: [] });
        } catch (e: any) {
          set({ error: e?.message || "Failed to clear chat" });
        }
      },

      // Error Explanation
      explainError: async (errorType: string, message: string, advice: string) => {
        try {
          const explanation = await tauriInvoke<ErrorExplanation>("ai_explain_error", {
            errorType,
            message,
            advice,
          });
          set({ currentExplanation: explanation, panelOpen: true, activeTab: "chat" });
          return explanation;
        } catch (_e: unknown) {
          // Fallback: structured error details
          const fallback: ErrorExplanation = {
            original_error: message,
            plain_language: `Error: ${message}`,
            possible_causes: ["Unable to determine cause - AI analysis unavailable"],
            suggested_fixes: [advice || "Try the operation again"],
            related_docs: [],
          };
          set({ currentExplanation: fallback });
          return fallback;
        }
      },

      // Suggestions
      generateSuggestions: async (context: Record<string, unknown>) => {
        set({ suggestionsLoading: true, error: null });
        try {
          const contextJson = JSON.stringify(context);
          const suggestions = await tauriInvoke<AiSuggestion[]>("ai_generate_suggestions", {
            contextJson,
          });
          set({ suggestions });
        } catch (e: any) {
          set({ error: e?.message || "Failed to generate suggestions" });
        } finally {
          set({ suggestionsLoading: false });
        }
      },

      loadSuggestions: async () => {
        set({ suggestionsLoading: true });
        try {
          const suggestions = await tauriInvoke<AiSuggestion[]>("ai_get_suggestions", undefined, []);
          set({ suggestions });
        } catch (e: any) {
          set({ error: e?.message || "Failed to load suggestions" });
        } finally {
          set({ suggestionsLoading: false });
        }
      },

      dismissSuggestion: async (id: string) => {
        try {
          await tauriInvoke("ai_dismiss_suggestion", { suggestionId: id });
          set((state) => ({
            suggestions: state.suggestions.filter((s) => s.id !== id),
          }));
        } catch (e: any) {
          set({ error: e?.message || "Failed to dismiss suggestion" });
        }
      },

      acceptSuggestion: async (id: string) => {
        try {
          const payload = await tauriInvoke<string>("ai_accept_suggestion", {
            suggestionId: id,
          });
          set((state) => ({
            suggestions: state.suggestions.filter((s) => s.id !== id),
          }));

          // Check if confirmation needed
          const needsConfirmation = await tauriInvoke<boolean>("ai_check_confirmation_needed", {
            actionPayload: payload,
          });
          if (needsConfirmation) {
            set({ confirmationPending: id });
          }

          return payload;
        } catch (e: any) {
          set({ error: e?.message || "Failed to accept suggestion" });
          throw e;
        }
      },

      // NL Job Creation
      parseNaturalLanguage: async (input: string) => {
        set({ nlLoading: true, error: null });
        try {
          const config = await tauriInvoke<ParsedJobConfig>("ai_parse_natural_language", {
            input,
          });
          set({ currentParsedJob: config });
          return config;
        } catch (e: any) {
          set({ error: e?.message || "Failed to parse natural language" });
          throw e;
        } finally {
          set({ nlLoading: false });
        }
      },

      // Safety Controls
      confirmAction: async (actionId: string) => {
        try {
          const confirmed = await tauriInvoke<boolean>("ai_confirm_action", { actionId });
          if (confirmed) {
            set({ confirmationPending: null });
          }
          return confirmed;
        } catch (e: any) {
          set({ error: e?.message || "Failed to confirm action" });
          return false;
        }
      },

      loadAuditLog: async (limit = 50, offset = 0) => {
        try {
          const log = await tauriInvoke<AiAuditEntry[]>("ai_get_audit_log", {
            limit,
            offset,
          });
          set({ auditLog: log });
        } catch (e: any) {
          set({ error: e?.message || "Failed to load audit log" });
        }
      },

      loadFeatureToggles: async () => {
        try {
          const toggles = await tauriInvoke<AiFeatureToggles>("ai_get_feature_toggles");
          set({ featureToggles: toggles });
        } catch (e: any) {
          set({ error: e?.message || "Failed to load feature toggles" });
        }
      },

      setFeatureToggles: async (toggles: AiFeatureToggles) => {
        try {
          await tauriInvoke("ai_set_feature_toggles", {
            togglesJson: JSON.stringify(toggles),
          });
          set({ featureToggles: toggles });
        } catch (e: any) {
          set({ error: e?.message || "Failed to update feature toggles" });
          throw e;
        }
      },

      // UI actions
      setPanelOpen: (open: boolean) => set({ panelOpen: open }),
      togglePanel: () => set((state) => ({ panelOpen: !state.panelOpen })),
      setActiveTab: (tab) => set({ activeTab: tab }),
      clearError: () => set({ error: null }),
      clearParsedJob: () => set({ currentParsedJob: null }),
    }),
    {
      name: "ufop-ai-state",
      partialize: (state) => ({
        panelOpen: state.panelOpen,
        activeTab: state.activeTab,
        featureToggles: state.featureToggles,
      }),
    },
  ),
);
