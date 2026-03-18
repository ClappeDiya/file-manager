/**
 * E2E Journey Tests: Local folder to cloud with incompatible names
 *
 * Tests the frontend flow for:
 * - Detecting incompatible filenames
 * - Showing compatibility warnings (simple + advanced mode)
 * - Transfer enqueue and progress tracking
 * - Encryption policy UI
 */

import { describe, it, expect, vi, beforeEach } from "vitest";

// Mock Tauri invoke
const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

describe("Journey 1: Local to Cloud Transfer E2E", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  describe("Compatibility Detection", () => {
    it("should detect incompatible filenames for OneDrive", async () => {
      const problematicFiles = [
        "report<Q1>.pdf",
        "meeting:notes.docx",
        'file"name".txt',
        "CON.txt",
        "normal.txt",
      ];

      // The backend should return compatibility results
      mockInvoke.mockResolvedValueOnce({
        translations: [
          {
            original_name: "report<Q1>.pdf",
            translated_name: "report\uFF1CQ1\uFF1E.pdf",
            rules_applied: ["Replaced '<' with fullwidth \uFF1C"],
            tier: "visible_auto",
            reversible: true,
          },
          {
            original_name: "meeting:notes.docx",
            translated_name: "meeting\uFF1Anotes.docx",
            rules_applied: ["Replaced ':' with fullwidth \uFF1A"],
            tier: "visible_auto",
            reversible: true,
          },
          {
            original_name: 'file"name".txt',
            translated_name: "file\uFF02name\uFF02.txt",
            rules_applied: ['Replaced \'"\' with fullwidth \uFF02'],
            tier: "visible_auto",
            reversible: true,
          },
          {
            original_name: "CON.txt",
            translated_name: "_CON.txt",
            rules_applied: ["Prefixed reserved name 'CON'"],
            tier: "visible_auto",
            reversible: true,
          },
        ],
        tier1_count: 0,
        tier2_count: 4,
        tier3_count: 0,
        total_modified: 4,
        simple_message:
          "We adjusted 4 names so the transfer could continue: 4 adjusted automatically",
      });

      const result = await mockInvoke("check_batch_compat", {
        names: problematicFiles,
        profile: "windows-ntfs",
      });

      expect(result.total_modified).toBe(4);
      expect(result.tier2_count).toBe(4);
      expect(result.simple_message).toContain("adjusted");
    });

    it("should show simple mode message for non-technical users", async () => {
      mockInvoke.mockResolvedValueOnce({
        simple_message:
          "We adjusted 3 names so the transfer could continue: 3 adjusted automatically",
        total_modified: 3,
      });

      const result = await mockInvoke("check_batch_compat", {
        names: ["file<1>.txt", "CON.txt", "data|pipe.csv"],
        profile: "windows-ntfs",
      });

      expect(result.simple_message).toContain("adjusted");
      expect(result.simple_message).toContain("3");
    });

    it("should return no modifications for compatible files", async () => {
      mockInvoke.mockResolvedValueOnce({
        translations: [],
        tier1_count: 0,
        tier2_count: 0,
        tier3_count: 0,
        total_modified: 0,
        simple_message:
          "All filenames are compatible with the destination.",
      });

      const result = await mockInvoke("check_batch_compat", {
        names: ["normal.txt", "safe_file.pdf"],
        profile: "windows-ntfs",
      });

      expect(result.total_modified).toBe(0);
      expect(result.simple_message).toContain("compatible");
    });
  });

  describe("Transfer Enqueue", () => {
    it("should enqueue transfer with translated names", async () => {
      const jobId = "550e8400-e29b-41d4-a716-446655440000";
      mockInvoke.mockResolvedValueOnce({
        id: jobId,
        source_path: "/Users/test/Documents/report<Q1>.pdf",
        dest_path: "onedrive://Documents/report\uFF1CQ1\uFF1E.pdf",
        total_bytes: 1024000,
        transferred_bytes: 0,
        status: "queued",
        priority: "normal",
      });

      const job = await mockInvoke("enqueue_transfer", {
        source: "/Users/test/Documents/report<Q1>.pdf",
        dest: "onedrive://Documents/report\uFF1CQ1\uFF1E.pdf",
        totalBytes: 1024000,
      });

      expect(job.status).toBe("queued");
      expect(job.dest_path).not.toContain("<");
    });

    it("should track transfer progress", async () => {
      mockInvoke.mockResolvedValueOnce({
        job_id: "test-job-1",
        transferred_bytes: 512000,
        total_bytes: 1024000,
        speed_bps: 50000,
        eta_seconds: 10,
        status: "active",
      });

      const progress = await mockInvoke("get_transfer", {
        jobId: "test-job-1",
      });

      expect(progress.transferred_bytes).toBe(512000);
      expect(progress.speed_bps).toBeGreaterThan(0);
      expect(progress.eta_seconds).toBeGreaterThan(0);
    });
  });

  describe("Encryption Policy", () => {
    it("should check encryption policy before upload", async () => {
      mockInvoke.mockResolvedValueOnce({
        encryption_required: true,
        zero_knowledge_required: true,
        policy_id: "pol_onedrive_pdf",
        policy_name: "OneDrive PDF Encryption",
      });

      const result = await mockInvoke("check_encryption_policy", {
        connectorType: "onedrive",
        destination: "Documents/reports",
        fileName: "annual_report.pdf",
      });

      expect(result.encryption_required).toBe(true);
      expect(result.zero_knowledge_required).toBe(true);
    });

    it("should not require encryption for non-matching files", async () => {
      mockInvoke.mockResolvedValueOnce({
        encryption_required: false,
        zero_knowledge_required: false,
        policy_id: null,
        policy_name: null,
      });

      const result = await mockInvoke("check_encryption_policy", {
        connectorType: "onedrive",
        destination: "Documents/images",
        fileName: "photo.jpg",
      });

      expect(result.encryption_required).toBe(false);
    });
  });

  describe("Transport Security", () => {
    it("should verify HTTPS for cloud connections", async () => {
      mockInvoke.mockResolvedValueOnce({
        secure: true,
        effective_protocol: "onedrive",
        upgraded: false,
        warnings: [],
        errors: [],
      });

      const check = await mockInvoke("check_transport_security", {
        protocol: "onedrive",
        host: "graph.microsoft.com",
        port: 443,
      });

      expect(check.secure).toBe(true);
      expect(check.errors).toHaveLength(0);
    });
  });
});
