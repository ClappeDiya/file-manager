/**
 * SSH Keyboard-Interactive Challenge Dialog
 *
 * Displays challenge prompts from SSH servers for MFA/OTP authentication.
 * Supports multiple prompts with echo/no-echo inputs.
 * 30-second timeout to prevent hanging.
 */
import { useState, useEffect, useRef, useCallback } from "react";

interface SshChallengePrompt {
  text: string;
  echo: boolean;
}

interface SshChallengeDialogProps {
  /** Whether the dialog is visible */
  isOpen: boolean;
  /** Server instruction text */
  instruction: string;
  /** Challenge prompts */
  prompts: SshChallengePrompt[];
  /** Called with responses when user submits */
  onSubmit: (responses: string[]) => void;
  /** Called when user cancels or timeout occurs */
  onCancel: () => void;
  /** Timeout in seconds (default: 30) */
  timeout?: number;
}

export function SshChallengeDialog({
  isOpen,
  instruction,
  prompts,
  onSubmit,
  onCancel,
  timeout = 30,
}: SshChallengeDialogProps) {
  const [responses, setResponses] = useState<string[]>([]);
  const [timeLeft, setTimeLeft] = useState(timeout);
  const firstInputRef = useRef<HTMLInputElement>(null);

  // Initialize responses array when prompts change
  useEffect(() => {
    setResponses(prompts.map(() => ""));
    setTimeLeft(timeout);
  }, [prompts, timeout]);

  // Focus first input when opened
  useEffect(() => {
    if (isOpen) {
      requestAnimationFrame(() => {
        firstInputRef.current?.focus();
      });
    }
  }, [isOpen]);

  // Countdown timer
  useEffect(() => {
    if (!isOpen) return;

    const interval = setInterval(() => {
      setTimeLeft((prev) => {
        if (prev <= 1) {
          onCancel();
          return 0;
        }
        return prev - 1;
      });
    }, 1000);

    return () => clearInterval(interval);
  }, [isOpen, onCancel]);

  const handleResponseChange = useCallback((index: number, value: string) => {
    setResponses((prev) => {
      const next = [...prev];
      next[index] = value;
      return next;
    });
  }, []);

  const handleSubmit = useCallback(() => {
    onSubmit(responses);
  }, [responses, onSubmit]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        handleSubmit();
      }
      if (e.key === "Escape") {
        e.preventDefault();
        onCancel();
      }
    },
    [handleSubmit, onCancel],
  );

  if (!isOpen) return null;

  const timerColor = timeLeft <= 10 ? "text-red-500" : timeLeft <= 20 ? "text-yellow-500" : "text-gray-400";

  return (
    <>
      {/* Backdrop */}
      <div className="fixed inset-0 z-50 bg-black/50" aria-hidden="true" />

      {/* Dialog */}
      <div
        className="fixed z-50 top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 w-[90vw] max-w-[440px] bg-white dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-xl shadow-xl"
        role="dialog"
        aria-label="SSH Authentication Challenge"
        onKeyDown={handleKeyDown}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-gray-200 dark:border-gray-700">
          <div className="flex items-center gap-2">
            <svg className="w-5 h-5 text-blue-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <rect x="3" y="11" width="18" height="11" rx="2" ry="2" />
              <path d="M7 11V7a5 5 0 0110 0v4" />
            </svg>
            <h3 className="text-sm font-semibold">SSH Authentication</h3>
          </div>
          <span className={`text-xs font-mono ${timerColor}`}>{timeLeft}s</span>
        </div>

        {/* Content */}
        <div className="px-5 py-4 space-y-4">
          {instruction && (
            <p className="text-sm text-gray-600 dark:text-gray-400">{instruction}</p>
          )}

          {prompts.map((prompt, index) => (
            <div key={index}>
              <label className="block text-xs font-medium mb-1">
                {prompt.text || `Response ${index + 1}`}
              </label>
              <input
                ref={index === 0 ? firstInputRef : undefined}
                type={prompt.echo ? "text" : "password"}
                value={responses[index] || ""}
                onChange={(e) => handleResponseChange(index, e.target.value)}
                className="w-full text-sm px-3 py-2 rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 font-mono focus:ring-2 focus:ring-blue-500 focus:border-blue-500 outline-none"
                placeholder={prompt.echo ? "Enter response" : "Enter code"}
                autoComplete="off"
              />
            </div>
          ))}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-2 px-5 py-3 border-t border-gray-200 dark:border-gray-700">
          <button
            onClick={onCancel}
            className="text-sm px-4 py-2 rounded-lg border border-gray-300 dark:border-gray-600 hover:bg-gray-50 dark:hover:bg-gray-800"
          >
            Cancel
          </button>
          <button
            onClick={handleSubmit}
            className="text-sm px-4 py-2 rounded-lg bg-blue-500 text-white hover:bg-blue-600"
          >
            Authenticate
          </button>
        </div>
      </div>
    </>
  );
}
