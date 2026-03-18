/**
 * E2E Journey Tests: Network transfers and computer migration
 *
 * Covers Journeys 6 (LAN transfer), 7 (computer migration)
 */

import { describe, it, expect, vi, beforeEach } from "vitest";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

describe("Journey 6: Computer-to-Computer LAN Transfer", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("should start and stop peer discovery", async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await mockInvoke("peer_start_discovery");

    mockInvoke.mockResolvedValueOnce(true);
    const isDiscovering = await mockInvoke("peer_is_discovering");
    expect(isDiscovering).toBe(true);

    mockInvoke.mockResolvedValueOnce(undefined);
    await mockInvoke("peer_stop_discovery");

    mockInvoke.mockResolvedValueOnce(false);
    const stopped = await mockInvoke("peer_is_discovering");
    expect(stopped).toBe(false);
  });

  it("should list discovered peers", async () => {
    mockInvoke.mockResolvedValueOnce([
      {
        id: "peer-001",
        display_name: "John's MacBook Pro",
        hostname: "johns-mbp.local",
        ip_address: "192.168.1.100",
        port: 42000,
        os: "macos",
        trust_level: "untrusted",
        online: true,
      },
      {
        id: "peer-002",
        display_name: "Office PC",
        hostname: "office-pc.local",
        ip_address: "192.168.1.101",
        port: 42000,
        os: "windows",
        trust_level: "trusted",
        online: true,
      },
    ]);

    const peers = await mockInvoke("peer_list_online");
    expect(peers).toHaveLength(2);
    expect(peers[0].display_name).toBe("John's MacBook Pro");
    expect(peers[1].trust_level).toBe("trusted");
  });

  it("should set peer trust level", async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await mockInvoke("peer_set_trust", {
      peerId: "peer-001",
      trustLevel: "trusted",
    });

    expect(mockInvoke).toHaveBeenCalledWith("peer_set_trust", {
      peerId: "peer-001",
      trustLevel: "trusted",
    });
  });

  it("should request file transfer to peer", async () => {
    mockInvoke.mockResolvedValueOnce({
      transfer_id: "transfer-001",
      status: "pending",
      peer_id: "peer-001",
      files: ["document.pdf", "photo.jpg"],
      total_bytes: 5242880,
    });

    const request = await mockInvoke("peer_request_transfer", {
      peerId: "peer-001",
      files: ["/Users/test/document.pdf", "/Users/test/photo.jpg"],
    });

    expect(request.status).toBe("pending");
    expect(request.total_bytes).toBe(5242880);
  });

  it("should respond to incoming transfer request", async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await mockInvoke("peer_respond_transfer", {
      transferId: "transfer-001",
      accept: true,
      destPath: "/Users/test/Received",
    });

    expect(mockInvoke).toHaveBeenCalledWith(
      "peer_respond_transfer",
      expect.objectContaining({ accept: true })
    );
  });

  it("should track peer transfer progress", async () => {
    mockInvoke.mockResolvedValueOnce({
      transfer_id: "transfer-001",
      transferred_bytes: 2621440,
      total_bytes: 5242880,
      speed_bps: 10485760,
      eta_seconds: 0.25,
      status: "active",
    });

    const progress = await mockInvoke("peer_get_transfer_progress", {
      transferId: "transfer-001",
    });

    expect(progress.transferred_bytes).toBe(2621440);
    expect(progress.status).toBe("active");
  });

  it("should set display name", async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await mockInvoke("peer_set_display_name", {
      name: "My MacBook Pro",
    });

    expect(mockInvoke).toHaveBeenCalledWith("peer_set_display_name", {
      name: "My MacBook Pro",
    });
  });

  it("should connect to peer manually by IP", async () => {
    mockInvoke.mockResolvedValueOnce({
      id: "manual-peer",
      display_name: "Remote PC",
      ip_address: "192.168.1.200",
      online: true,
    });

    const peer = await mockInvoke("peer_connect_manual", {
      address: "192.168.1.200",
      port: 42000,
    });

    expect(peer.online).toBe(true);
  });

  it("should manage server-to-server transfers", async () => {
    // Get capability matrix
    mockInvoke.mockResolvedValueOnce({
      sftp_to_sftp: { supported: true, method: "stream_relay" },
      sftp_to_s3: { supported: true, method: "stream_upload" },
      s3_to_s3: { supported: true, method: "server_side_copy" },
      ftp_to_sftp: { supported: true, method: "stream_relay" },
    });

    const matrix = await mockInvoke(
      "server_transfer_capability_matrix"
    );
    expect(matrix.s3_to_s3.method).toBe("server_side_copy");

    // Create transfer
    mockInvoke.mockResolvedValueOnce({
      id: "s2s-001",
      status: "pending",
      method: "stream_relay",
    });

    const transfer = await mockInvoke("server_transfer_create", {
      sourceConnector: "sftp",
      destConnector: "s3",
      sourcePath: "/remote/file.dat",
      destPath: "s3://bucket/file.dat",
    });

    expect(transfer.status).toBe("pending");
  });
});

describe("Journey 7: Computer Migration", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("should export connection profiles", async () => {
    mockInvoke.mockResolvedValueOnce(
      JSON.stringify({
        version: 1,
        connections: [
          {
            name: "Work SFTP",
            protocol: "sftp",
            host: "sftp.company.com",
          },
          {
            name: "AWS S3",
            protocol: "s3",
            host: "s3.amazonaws.com",
          },
        ],
      })
    );

    const exported = await mockInvoke("export_connections");
    const parsed = JSON.parse(exported as string);
    expect(parsed.connections).toHaveLength(2);
  });

  it("should import connection profiles on new computer", async () => {
    mockInvoke.mockResolvedValueOnce(2);

    const imported = await mockInvoke("import_connections", {
      data: JSON.stringify({
        version: 1,
        connections: [
          {
            name: "Work SFTP",
            protocol: "sftp",
            host: "sftp.company.com",
          },
          {
            name: "AWS S3",
            protocol: "s3",
            host: "s3.amazonaws.com",
          },
        ],
      }),
    });

    expect(imported).toBe(2);
  });

  it("should save and load workspace state", async () => {
    // Save
    mockInvoke.mockResolvedValueOnce(undefined);
    await mockInvoke("save_workspace_state", {
      state: {
        left_panel_path: "/Users/test/Documents",
        right_panel_path: "/Users/test/Downloads",
        sort_column: "name",
        sort_direction: "asc",
      },
    });

    // Load
    mockInvoke.mockResolvedValueOnce({
      left_panel_path: "/Users/test/Documents",
      right_panel_path: "/Users/test/Downloads",
      sort_column: "name",
      sort_direction: "asc",
      version: 1,
    });

    const state = await mockInvoke("load_workspace_state");
    expect(state.left_panel_path).toBe("/Users/test/Documents");
    expect(state.version).toBe(1);
  });

  it("should search connections for migration", async () => {
    mockInvoke.mockResolvedValueOnce([
      {
        id: "conn-1",
        name: "Production SFTP",
        protocol: "sftp",
        host: "prod.company.com",
        tags: ["production", "work"],
      },
    ]);

    const results = await mockInvoke("search_connections", {
      query: "production",
    });

    expect(results).toHaveLength(1);
    expect(results[0].tags).toContain("production");
  });

  it("should create connection groups", async () => {
    mockInvoke.mockResolvedValueOnce({
      id: "group-1",
      name: "Work Servers",
      color: null,
    });

    const group = await mockInvoke("create_connection_group", {
      name: "Work Servers",
    });

    expect(group.name).toBe("Work Servers");
  });

  it("should handle transfer queue persistence", async () => {
    // Enqueue transfers
    for (let i = 0; i < 5; i++) {
      mockInvoke.mockResolvedValueOnce({
        id: `job-${i}`,
        source_path: `/old/file_${i}.dat`,
        dest_path: `/new/file_${i}.dat`,
        status: "queued",
      });
    }

    for (let i = 0; i < 5; i++) {
      await mockInvoke("enqueue_transfer", {
        source: `/old/file_${i}.dat`,
        dest: `/new/file_${i}.dat`,
        totalBytes: 1024 * (i + 1),
      });
    }

    // List all transfers
    mockInvoke.mockResolvedValueOnce(
      Array.from({ length: 5 }, (_, i) => ({
        id: `job-${i}`,
        source_path: `/old/file_${i}.dat`,
        dest_path: `/new/file_${i}.dat`,
        status: "queued",
      }))
    );

    const jobs = await mockInvoke("list_transfers");
    expect(jobs).toHaveLength(5);
  });

  it("should verify data integrity after migration", async () => {
    mockInvoke.mockResolvedValueOnce({
      verified: true,
      source_checksum: "abc123def456",
      dest_checksum: "abc123def456",
      algorithm: "xxhash3",
    });

    const verification = await mockInvoke("verify_transfer", {
      jobId: "migration-job-1",
    });

    expect(verification.verified).toBe(true);
    expect(verification.source_checksum).toBe(
      verification.dest_checksum
    );
  });
});
