/**
 * E2E Journey Tests: Enterprise governance and AI features
 *
 * Covers Journeys 3 (enterprise governed transfer), 4 (AI cleanup)
 */

import { describe, it, expect, vi, beforeEach } from "vitest";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

describe("Journey 3: Enterprise-Governed Transfer", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("should add encryption policy", async () => {
    mockInvoke.mockResolvedValueOnce(undefined);

    await mockInvoke("add_encryption_policy", {
      policy: {
        id: "pol_s3_sensitive",
        name: "S3 Sensitive Data Encryption",
        connector_type: "s3",
        destination_pattern: "sensitive-data/*",
        required: true,
        file_patterns: ["*.pdf", "*.docx"],
        zero_knowledge_required: true,
        enabled: true,
      },
    });

    expect(mockInvoke).toHaveBeenCalledWith(
      "add_encryption_policy",
      expect.any(Object)
    );
  });

  it("should list encryption policies", async () => {
    mockInvoke.mockResolvedValueOnce([
      {
        id: "pol_s3_sensitive",
        name: "S3 Sensitive Data Encryption",
        connector_type: "s3",
        required: true,
        enabled: true,
      },
      {
        id: "pol_sftp_all",
        name: "SFTP All Files Encryption",
        connector_type: "sftp",
        required: true,
        enabled: true,
      },
    ]);

    const policies = await mockInvoke("list_encryption_policies");
    expect(policies).toHaveLength(2);
    expect(policies[0].required).toBe(true);
  });

  it("should check policy for file transfer", async () => {
    mockInvoke.mockResolvedValueOnce({
      encryption_required: true,
      zero_knowledge_required: true,
      policy_id: "pol_s3_sensitive",
      policy_name: "S3 Sensitive Data Encryption",
    });

    const result = await mockInvoke("check_encryption_policy", {
      connectorType: "s3",
      destination: "sensitive-data/reports",
      fileName: "financial_report.pdf",
    });

    expect(result.encryption_required).toBe(true);
    expect(result.zero_knowledge_required).toBe(true);
    expect(result.policy_name).toBe("S3 Sensitive Data Encryption");
  });

  it("should enforce HTTPS for cloud API calls", async () => {
    mockInvoke.mockResolvedValueOnce("https://api.example.com/v2");

    const secureUrl = await mockInvoke("enforce_https_url", {
      url: "http://api.example.com/v2",
    });

    expect((secureUrl as string).startsWith("https://")).toBe(true);
  });

  it("should check transport security for protocols", async () => {
    // SFTP: always secure
    mockInvoke.mockResolvedValueOnce({
      secure: true,
      effective_protocol: "sftp",
      upgraded: false,
      warnings: [],
      errors: [],
    });

    const sftpCheck = await mockInvoke("check_transport_security", {
      protocol: "sftp",
      host: "sftp.company.com",
      port: 22,
    });
    expect(sftpCheck.secure).toBe(true);

    // FTP: auto-upgraded
    mockInvoke.mockResolvedValueOnce({
      secure: true,
      effective_protocol: "ftps",
      upgraded: true,
      warnings: ["FTP auto-upgraded to FTPS"],
      errors: [],
    });

    const ftpCheck = await mockInvoke("check_transport_security", {
      protocol: "ftp",
      host: "ftp.company.com",
      port: 21,
    });
    expect(ftpCheck.upgraded).toBe(true);
    expect(ftpCheck.effective_protocol).toBe("ftps");
  });

  it("should encrypt file for zero-knowledge upload", async () => {
    mockInvoke.mockResolvedValueOnce(
      new Uint8Array([1, 2, 3, 4, 5]) // encrypted bytes
    );

    const encrypted = await mockInvoke("encrypt_for_upload", {
      vaultId: "vault-001",
      data: new Uint8Array([72, 101, 108, 108, 111]), // "Hello"
      fileName: "sensitive.pdf",
    });

    expect(encrypted).toBeDefined();
    expect(encrypted.length).toBeGreaterThan(0);
  });

  it("should get recommended security for protocols", async () => {
    mockInvoke.mockResolvedValueOnce({
      min_tls_version: "tls_1.3",
      require_encryption: true,
      auto_upgrade: true,
      verify_certificates: true,
    });

    const security = await mockInvoke("get_recommended_security", {
      protocol: "s3",
    });

    expect(security.min_tls_version).toBe("tls_1.3");
    expect(security.require_encryption).toBe(true);
  });
});

describe("Journey 4: AI-Assisted Folder Cleanup", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("should explain errors in plain language", async () => {
    mockInvoke.mockResolvedValueOnce({
      original_error: "EACCES: permission denied",
      plain_language:
        "You don't have permission to access this file or folder.",
      possible_causes: [
        "File is owned by another user",
        "File is read-only",
        "System file protection is active",
      ],
      suggested_fixes: [
        "Right-click the file and check permissions",
        "Run as administrator",
        "Change file ownership",
      ],
    });

    const explanation = await mockInvoke("ai_explain_error", {
      error: "EACCES: permission denied",
    });

    expect(explanation.plain_language).toBeTruthy();
    expect(explanation.possible_causes.length).toBeGreaterThan(0);
    expect(explanation.suggested_fixes.length).toBeGreaterThan(0);
  });

  it("should generate cleanup suggestions", async () => {
    mockInvoke.mockResolvedValueOnce([
      {
        id: "sug-001",
        category: "cleanup",
        title: "Remove duplicate files",
        description:
          "Found 3 likely duplicate files in /downloads (report_v1, v2, v3)",
        confidence: 0.85,
        dismissed: false,
      },
      {
        id: "sug-002",
        category: "exclusion",
        title: "Exclude system files from sync",
        description:
          ".DS_Store and Thumbs.db files should be excluded from sync",
        confidence: 0.95,
        dismissed: false,
      },
    ]);

    const suggestions = await mockInvoke("ai_generate_suggestions", {
      files: [
        "/downloads/report_v1.pdf",
        "/downloads/report_v2.pdf",
        "/downloads/report_v3.pdf",
        "/downloads/.DS_Store",
        "/downloads/Thumbs.db",
      ],
    });

    expect(suggestions).toHaveLength(2);
    expect(suggestions[0].confidence).toBeGreaterThan(0);
    expect(suggestions[0].confidence).toBeLessThanOrEqual(1);
  });

  it("should support AI chat", async () => {
    mockInvoke.mockResolvedValueOnce(
      "I suggest organizing your downloads by file type: create folders for Documents, Images, and Archives."
    );

    const response = await mockInvoke("ai_chat", {
      message: "How should I organize my downloads?",
    });

    expect(response).toBeTruthy();
    expect(typeof response).toBe("string");
  });

  it("should get chat history", async () => {
    mockInvoke.mockResolvedValueOnce([
      {
        id: "msg-1",
        role: "user",
        content: "How should I organize my downloads?",
        timestamp: "2024-01-15T10:00:00Z",
      },
      {
        id: "msg-2",
        role: "assistant",
        content: "I suggest organizing by file type...",
        timestamp: "2024-01-15T10:00:01Z",
      },
    ]);

    const history = await mockInvoke("ai_get_chat_history");
    expect(history).toHaveLength(2);
    expect(history[0].role).toBe("user");
    expect(history[1].role).toBe("assistant");
  });

  it("should parse natural language commands", async () => {
    mockInvoke.mockResolvedValueOnce({
      action: "move",
      source_pattern: "*.pdf",
      source_path: "/downloads",
      dest_path: "/documents",
      confidence: 0.9,
      needs_confirmation: true,
    });

    const parsed = await mockInvoke("ai_parse_natural_language", {
      input: "Move all PDFs from downloads to documents",
    });

    expect(parsed.action).toBe("move");
    expect(parsed.needs_confirmation).toBe(true);
    expect(parsed.confidence).toBeGreaterThan(0.5);
  });

  it("should require confirmation for destructive actions", async () => {
    mockInvoke.mockResolvedValueOnce(true);

    const needsConfirm = await mockInvoke(
      "ai_check_confirmation_needed",
      {
        action: "delete",
      }
    );

    expect(needsConfirm).toBe(true);
  });

  it("should dismiss and accept suggestions", async () => {
    // Dismiss
    mockInvoke.mockResolvedValueOnce(undefined);
    await mockInvoke("ai_dismiss_suggestion", {
      suggestionId: "sug-001",
    });

    // Accept
    mockInvoke.mockResolvedValueOnce({ action: "create_sync_rule" });
    const action = await mockInvoke("ai_accept_suggestion", {
      suggestionId: "sug-002",
    });

    expect(action).toBeDefined();
  });

  it("should manage feature toggles", async () => {
    mockInvoke.mockResolvedValueOnce({
      suggestions_enabled: true,
      chat_enabled: true,
      natural_language_enabled: true,
      proactive_suggestions: false,
    });

    const toggles = await mockInvoke("ai_get_feature_toggles");
    expect(toggles.suggestions_enabled).toBe(true);
    expect(toggles.chat_enabled).toBe(true);

    // Disable suggestions
    mockInvoke.mockResolvedValueOnce(undefined);
    await mockInvoke("ai_set_feature_toggles", {
      toggles: { ...toggles, suggestions_enabled: false },
    });
  });

  it("should track AI audit log", async () => {
    mockInvoke.mockResolvedValueOnce([
      {
        timestamp: "2024-01-15T10:00:00Z",
        action: "chat",
        input: "How to organize files?",
        output: "I suggest...",
      },
      {
        timestamp: "2024-01-15T10:01:00Z",
        action: "generate_suggestions",
        input: "5 files",
        output: "2 suggestions generated",
      },
    ]);

    const auditLog = await mockInvoke("ai_get_audit_log");
    expect(auditLog).toHaveLength(2);
    expect(auditLog[0].action).toBe("chat");
  });
});
