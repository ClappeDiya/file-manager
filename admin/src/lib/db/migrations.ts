/**
 * Versioned schema. Each migration runs once, in a transaction, and bumps
 * `PRAGMA user_version`. v1 also upgrades databases created by the old
 * unversioned store (tables existed, some columns were missing).
 */
import type Database from 'better-sqlite3';

export interface Migration {
  version: number;
  description: string;
  up: (d: Database.Database) => void;
}

function hasColumn(d: Database.Database, table: string, column: string): boolean {
  return (d.prepare(`PRAGMA table_info(${table})`).all() as { name: string }[]).some((c) => c.name === column);
}

function addColumn(d: Database.Database, table: string, column: string, def: string) {
  if (!hasColumn(d, table, column)) d.exec(`ALTER TABLE ${table} ADD COLUMN ${column} ${def}`);
}

export const MIGRATIONS: Migration[] = [
  {
    version: 1,
    description: 'Base tables (users, policies, approvals, audit, devices, workspaces)',
    up(d) {
      d.exec(`
        CREATE TABLE IF NOT EXISTS users (
          id TEXT PRIMARY KEY, email TEXT UNIQUE NOT NULL, name TEXT NOT NULL DEFAULT '',
          role TEXT NOT NULL DEFAULT 'user', password_hash TEXT, permissions TEXT DEFAULT '[]',
          organization_id TEXT DEFAULT '', last_login_at TEXT,
          created_at TEXT NOT NULL DEFAULT (datetime('now')), is_active INTEGER NOT NULL DEFAULT 1,
          sso_provider TEXT, mfa_enabled INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS policies (
          id TEXT PRIMARY KEY, domain TEXT NOT NULL, name TEXT NOT NULL,
          assignment_target TEXT NOT NULL DEFAULT 'organization', assignment_value TEXT DEFAULT '',
          enforcement_mode TEXT NOT NULL DEFAULT 'enforce', configuration TEXT NOT NULL DEFAULT '{}',
          created_by TEXT DEFAULT '', version INTEGER NOT NULL DEFAULT 1, is_active INTEGER NOT NULL DEFAULT 1,
          created_at TEXT NOT NULL DEFAULT (datetime('now')), updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS approvals (
          id TEXT PRIMARY KEY, operation TEXT NOT NULL, trigger_type TEXT DEFAULT '', source TEXT DEFAULT '',
          destination TEXT DEFAULT '', file_count INTEGER DEFAULT 0, total_size INTEGER DEFAULT 0,
          state TEXT NOT NULL DEFAULT 'pending', requested_by TEXT DEFAULT '',
          requested_at TEXT NOT NULL DEFAULT (datetime('now')), reviewed_by TEXT, reviewed_at TEXT,
          review_comment TEXT, expires_at TEXT
        );
        CREATE TABLE IF NOT EXISTS audit_entries (
          id TEXT PRIMARY KEY, event_type TEXT NOT NULL, severity TEXT NOT NULL DEFAULT 'info',
          timestamp TEXT NOT NULL DEFAULT (datetime('now')), user_id TEXT DEFAULT '', user_email TEXT DEFAULT '',
          ip_address TEXT DEFAULT '', user_agent TEXT DEFAULT '', description TEXT NOT NULL DEFAULT '',
          metadata TEXT DEFAULT '{}', integrity_hash TEXT DEFAULT '', previous_hash TEXT DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS devices (
          id TEXT PRIMARY KEY, device_id TEXT UNIQUE NOT NULL, device_name TEXT NOT NULL DEFAULT '',
          user_id TEXT DEFAULT '', user_email TEXT DEFAULT '', client_version TEXT DEFAULT '', os TEXT DEFAULT '',
          arch TEXT DEFAULT '', status TEXT NOT NULL DEFAULT 'offline', last_seen TEXT, ip_address TEXT DEFAULT '',
          failure_patterns TEXT DEFAULT '[]', policy_compliance TEXT DEFAULT '{}', cpu_usage REAL DEFAULT 0,
          memory_usage REAL DEFAULT 0, disk_usage REAL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS workspaces (
          id TEXT PRIMARY KEY, name TEXT NOT NULL, organization_id TEXT DEFAULT '', connections TEXT DEFAULT '[]',
          sync_templates TEXT DEFAULT '[]', automation_templates TEXT DEFAULT '[]', members TEXT DEFAULT '[]',
          created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS shared_connections (
          id TEXT PRIMARY KEY, name TEXT NOT NULL, connector_type TEXT NOT NULL DEFAULT '', config TEXT DEFAULT '{}',
          owner_id TEXT DEFAULT '', shared_with TEXT DEFAULT '[]', propagation_status TEXT DEFAULT 'active',
          created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS sync_templates (
          id TEXT PRIMARY KEY, name TEXT NOT NULL, source_connector TEXT DEFAULT '', source_path TEXT DEFAULT '',
          dest_connector TEXT DEFAULT '', dest_path TEXT DEFAULT '', sync_mode TEXT DEFAULT 'one_way',
          conflict_resolution TEXT DEFAULT 'newest_wins', schedule TEXT DEFAULT '', activated_by_users TEXT DEFAULT '[]',
          propagation_status TEXT DEFAULT 'active', created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
      `);
      // Columns the old store read or wrote but never created.
      addColumn(d, 'policies', 'description', "TEXT NOT NULL DEFAULT ''");
      addColumn(d, 'approvals', 'reason', "TEXT NOT NULL DEFAULT ''");
      addColumn(d, 'approvals', 'triggering_policy', "TEXT NOT NULL DEFAULT ''");
      addColumn(d, 'approvals', 'requested_by_name', "TEXT NOT NULL DEFAULT ''");
      addColumn(d, 'approvals', 'organization_id', "TEXT NOT NULL DEFAULT ''");
      addColumn(d, 'devices', 'user_name', "TEXT NOT NULL DEFAULT ''");
      addColumn(d, 'devices', 'os_version', "TEXT NOT NULL DEFAULT ''");
      addColumn(d, 'policies', 'org_id', "TEXT NOT NULL DEFAULT 'org_default'");
      addColumn(d, 'approvals', 'reviewed_by_name', "TEXT NOT NULL DEFAULT ''");
    },
  },
  {
    version: 2,
    description: 'Organizations, memberships, groups, invitations',
    up(d) {
      d.exec(`
        CREATE TABLE organizations (
          id TEXT PRIMARY KEY, name TEXT NOT NULL, parent_id TEXT REFERENCES organizations(id),
          detached INTEGER NOT NULL DEFAULT 0, protected INTEGER NOT NULL DEFAULT 0,
          status TEXT NOT NULL DEFAULT 'active', license_plan TEXT, license_seats INTEGER,
          offboard_after TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL
        );
        CREATE INDEX idx_orgs_parent ON organizations(parent_id);
        CREATE TABLE org_memberships (
          user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
          org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          role TEXT NOT NULL, created_at TEXT NOT NULL, PRIMARY KEY (user_id, org_id)
        );
        CREATE TABLE groups (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', org_role TEXT,
          delegate_to_children INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL, UNIQUE(org_id, name)
        );
        CREATE TABLE group_members (
          group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
          user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
          role TEXT NOT NULL DEFAULT 'member', added_at TEXT NOT NULL, PRIMARY KEY (group_id, user_id)
        );
        CREATE TABLE group_access (
          group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
          resource_type TEXT NOT NULL, resource_id TEXT NOT NULL,
          PRIMARY KEY (group_id, resource_type, resource_id)
        );
        CREATE TABLE invitations (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          email TEXT NOT NULL, role TEXT NOT NULL, group_id TEXT, group_role TEXT NOT NULL DEFAULT 'member',
          token_hash TEXT NOT NULL UNIQUE, invited_by TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL,
          expires_at TEXT NOT NULL, accepted_at TEXT, accepted_user_id TEXT, revoked_at TEXT
        );
        CREATE INDEX idx_invites_org ON invitations(org_id);
      `);
      const ts = new Date().toISOString();
      d.prepare(`INSERT INTO organizations (id, name, created_at, updated_at) VALUES ('org_default', 'Default organization', ?, ?)`).run(ts, ts);
      // Existing users become members of their organization (or the default one).
      const orgs = d.prepare(`SELECT DISTINCT organization_id AS o FROM users WHERE organization_id NOT IN ('', 'org_default')`).all() as { o: string }[];
      for (const { o } of orgs) d.prepare(`INSERT OR IGNORE INTO organizations (id, name, created_at, updated_at) VALUES (?, ?, ?, ?)`).run(o, o, ts, ts);
      d.prepare(`UPDATE users SET organization_id = 'org_default' WHERE organization_id IS NULL OR organization_id = ''`).run();
      d.prepare(`INSERT OR IGNORE INTO org_memberships (user_id, org_id, role, created_at) SELECT id, organization_id, role, ? FROM users`).run(ts);
    },
  },
  {
    version: 3,
    description: 'Fleet: machines, enrollment, reports, commands, fleet policies, credentials',
    up(d) {
      d.exec(`
        CREATE TABLE machines (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          name TEXT NOT NULL, hostname TEXT NOT NULL DEFAULT '', os TEXT NOT NULL DEFAULT '',
          os_version TEXT NOT NULL DEFAULT '', arch TEXT NOT NULL DEFAULT '', client_version TEXT NOT NULL DEFAULT '',
          agent_kind TEXT NOT NULL DEFAULT 'desktop', status TEXT NOT NULL DEFAULT 'active', heartbeat_seconds INTEGER NOT NULL DEFAULT 60,
          token_hash TEXT UNIQUE, token_rotated_at TEXT, registered_at TEXT NOT NULL, registered_via TEXT NOT NULL DEFAULT '',
          last_seen_at TEXT, last_ip TEXT NOT NULL DEFAULT '', last_report_at TEXT,
          tags TEXT NOT NULL DEFAULT '[]', notes TEXT NOT NULL DEFAULT '',
          applied_policies TEXT NOT NULL DEFAULT '{}', jobs TEXT NOT NULL DEFAULT '[]', revoked_at TEXT
        );
        CREATE INDEX idx_machines_org ON machines(org_id);
        CREATE TABLE tag_colors (
          org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE, tag TEXT NOT NULL, color TEXT NOT NULL,
          PRIMARY KEY (org_id, tag)
        );
        CREATE TABLE registration_links (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          label TEXT NOT NULL DEFAULT '', token_hash TEXT NOT NULL UNIQUE, token_prefix TEXT NOT NULL, token_ciphertext TEXT NOT NULL,
          created_by TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL, expires_at TEXT, max_uses INTEGER,
          uses INTEGER NOT NULL DEFAULT 0, revoked_at TEXT, default_tags TEXT NOT NULL DEFAULT '[]'
        );
        CREATE TABLE machine_claims (
          id TEXT PRIMARY KEY, code TEXT NOT NULL UNIQUE, secret_hash TEXT NOT NULL,
          hostname TEXT NOT NULL DEFAULT '', os TEXT NOT NULL DEFAULT '', client_version TEXT NOT NULL DEFAULT '',
          agent_kind TEXT NOT NULL DEFAULT 'desktop', created_at TEXT NOT NULL, expires_at TEXT NOT NULL,
          status TEXT NOT NULL DEFAULT 'pending', machine_id TEXT, org_id TEXT, claimed_by TEXT, pending_token TEXT
        );
        CREATE TABLE backup_reports (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL, machine_id TEXT NOT NULL REFERENCES machines(id) ON DELETE CASCADE,
          backup_id TEXT NOT NULL, backup_name TEXT NOT NULL DEFAULT '', operation TEXT NOT NULL DEFAULT 'backup',
          status TEXT NOT NULL, started_at TEXT, finished_at TEXT NOT NULL, duration_ms INTEGER NOT NULL DEFAULT 0,
          files_examined INTEGER NOT NULL DEFAULT 0, files_added INTEGER NOT NULL DEFAULT 0,
          files_modified INTEGER NOT NULL DEFAULT 0, files_deleted INTEGER NOT NULL DEFAULT 0,
          bytes_uploaded INTEGER NOT NULL DEFAULT 0, source_size INTEGER NOT NULL DEFAULT 0,
          backend_size INTEGER NOT NULL DEFAULT 0, version_count INTEGER NOT NULL DEFAULT 0,
          warnings INTEGER NOT NULL DEFAULT 0, errors INTEGER NOT NULL DEFAULT 0, message TEXT NOT NULL DEFAULT '',
          received_at TEXT NOT NULL, raw TEXT NOT NULL DEFAULT '{}'
        );
        CREATE INDEX idx_reports_backup ON backup_reports(machine_id, backup_id, finished_at);
        CREATE INDEX idx_reports_org ON backup_reports(org_id, finished_at);
        CREATE TABLE backups (
          org_id TEXT NOT NULL, machine_id TEXT NOT NULL REFERENCES machines(id) ON DELETE CASCADE,
          backup_id TEXT NOT NULL, name TEXT NOT NULL DEFAULT '', last_status TEXT, last_run_at TEXT,
          last_success_at TEXT, schedule_interval_minutes INTEGER, source_size INTEGER NOT NULL DEFAULT 0,
          backend_size INTEGER NOT NULL DEFAULT 0, version_count INTEGER NOT NULL DEFAULT 0,
          PRIMARY KEY (machine_id, backup_id)
        );
        CREATE TABLE agent_commands (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL, machine_id TEXT NOT NULL REFERENCES machines(id) ON DELETE CASCADE,
          kind TEXT NOT NULL, payload TEXT NOT NULL DEFAULT '{}', status TEXT NOT NULL DEFAULT 'queued',
          result TEXT, created_by TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL, delivered_at TEXT,
          completed_at TEXT, expires_at TEXT NOT NULL
        );
        CREATE INDEX idx_commands_machine ON agent_commands(machine_id, status);
        CREATE TABLE fleet_policies (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', target TEXT NOT NULL DEFAULT '{}',
          job TEXT NOT NULL DEFAULT '{}', credential_ids TEXT NOT NULL DEFAULT '[]', enabled INTEGER NOT NULL DEFAULT 1,
          version INTEGER NOT NULL DEFAULT 1, created_by TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
        CREATE TABLE central_credentials (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          name TEXT NOT NULL, kind TEXT NOT NULL DEFAULT 'secret', description TEXT NOT NULL DEFAULT '',
          ciphertext TEXT NOT NULL, created_by TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL, UNIQUE(org_id, name)
        );
      `);
    },
  },
  {
    version: 4,
    description: 'Alerts, notification channels, AI analysis',
    up(d) {
      d.exec(`
        CREATE TABLE notification_channels (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          name TEXT NOT NULL, kind TEXT NOT NULL, config TEXT NOT NULL DEFAULT '{}', secret_ciphertext TEXT,
          enabled INTEGER NOT NULL DEFAULT 1, created_at TEXT NOT NULL, last_error TEXT, last_sent_at TEXT
        );
        CREATE TABLE alert_rules (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          name TEXT NOT NULL, kind TEXT NOT NULL, params TEXT NOT NULL DEFAULT '{}', scope TEXT NOT NULL DEFAULT '{}',
          channel_ids TEXT NOT NULL DEFAULT '[]', enabled INTEGER NOT NULL DEFAULT 1,
          cooldown_minutes INTEGER NOT NULL DEFAULT 360, created_at TEXT NOT NULL
        );
        CREATE TABLE alerts (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL, rule_id TEXT REFERENCES alert_rules(id) ON DELETE SET NULL,
          machine_id TEXT, backup_id TEXT, kind TEXT NOT NULL, severity TEXT NOT NULL DEFAULT 'warning',
          message TEXT NOT NULL, details TEXT NOT NULL DEFAULT '{}', fingerprint TEXT NOT NULL,
          created_at TEXT NOT NULL, last_notified_at TEXT, resolved_at TEXT, notified TEXT NOT NULL DEFAULT '[]'
        );
        CREATE INDEX idx_alerts_open ON alerts(org_id, resolved_at);
        CREATE INDEX idx_alerts_fp ON alerts(fingerprint, resolved_at);
        CREATE TABLE ai_settings (
          org_id TEXT PRIMARY KEY REFERENCES organizations(id) ON DELETE CASCADE,
          enabled INTEGER NOT NULL DEFAULT 0, model TEXT NOT NULL DEFAULT '', instructions TEXT NOT NULL DEFAULT '',
          schedule TEXT NOT NULL DEFAULT 'off', channel_ids TEXT NOT NULL DEFAULT '[]', api_key_ciphertext TEXT,
          api_base TEXT NOT NULL DEFAULT '', last_run_at TEXT
        );
        CREATE TABLE ai_reports (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          kind TEXT NOT NULL, scope TEXT NOT NULL DEFAULT '{}', model TEXT NOT NULL DEFAULT '',
          input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
          content TEXT NOT NULL DEFAULT '', error TEXT, created_by TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL
        );
      `);
    },
  },
  {
    version: 5,
    description: 'Single sign-on, linked identities, MFA',
    up(d) {
      d.exec(`
        CREATE TABLE sso_providers (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          name TEXT NOT NULL, kind TEXT NOT NULL, config TEXT NOT NULL DEFAULT '{}', secret_ciphertext TEXT,
          default_role TEXT NOT NULL DEFAULT 'user', default_group_id TEXT, require_mfa INTEGER NOT NULL DEFAULT 0,
          allow_jit INTEGER NOT NULL DEFAULT 1, domains TEXT NOT NULL DEFAULT '[]', enabled INTEGER NOT NULL DEFAULT 1,
          created_at TEXT NOT NULL
        );
        CREATE TABLE user_identities (
          id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
          provider_id TEXT NOT NULL REFERENCES sso_providers(id) ON DELETE CASCADE, subject TEXT NOT NULL,
          email TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL, last_used_at TEXT, UNIQUE(provider_id, subject)
        );
        CREATE TABLE auth_states (
          state TEXT PRIMARY KEY, provider_id TEXT NOT NULL, nonce TEXT NOT NULL, code_verifier TEXT NOT NULL,
          redirect TEXT NOT NULL DEFAULT '/', link_user_id TEXT, created_at TEXT NOT NULL
        );
        CREATE TABLE saml_requests (id TEXT PRIMARY KEY, value TEXT NOT NULL, created_at TEXT NOT NULL);
        CREATE TABLE mfa_challenges (
          id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
          org_id TEXT, method TEXT NOT NULL DEFAULT 'password', attempts INTEGER NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL, expires_at TEXT NOT NULL
        );
      `);
      addColumn(d, 'users', 'totp_secret_ciphertext', 'TEXT');
      addColumn(d, 'users', 'totp_pending_ciphertext', 'TEXT');
      addColumn(d, 'users', 'recovery_codes', "TEXT NOT NULL DEFAULT '[]'");
      addColumn(d, 'users', 'last_totp_step', 'INTEGER');
    },
  },
  {
    version: 6,
    description: 'Append-only, hash-chained audit log',
    up(d) {
      addColumn(d, 'audit_entries', 'sequence', 'INTEGER');
      addColumn(d, 'audit_entries', 'org_id', "TEXT NOT NULL DEFAULT ''");
      d.exec(`
        CREATE UNIQUE INDEX IF NOT EXISTS idx_audit_seq ON audit_entries(sequence);
        CREATE TRIGGER audit_no_update BEFORE UPDATE ON audit_entries
          BEGIN SELECT RAISE(ABORT, 'audit entries are append-only'); END;
        CREATE TRIGGER audit_no_delete BEFORE DELETE ON audit_entries
          BEGIN SELECT RAISE(ABORT, 'audit entries are append-only'); END;
        CREATE TABLE audit_anchors (
          id TEXT PRIMARY KEY, last_sequence INTEGER NOT NULL, last_hash TEXT NOT NULL,
          entry_count INTEGER NOT NULL, created_at TEXT NOT NULL, signature TEXT NOT NULL
        );
        CREATE TRIGGER audit_anchor_no_update BEFORE UPDATE ON audit_anchors
          BEGIN SELECT RAISE(ABORT, 'audit anchors are append-only'); END;
        CREATE TRIGGER audit_anchor_no_delete BEFORE DELETE ON audit_anchors
          BEGIN SELECT RAISE(ABORT, 'audit anchors are append-only'); END;
      `);
    },
  },
  {
    version: 7,
    description: 'AI governance settings per organization',
    up(d) {
      d.exec(`
        CREATE TABLE ai_governance (
          org_id TEXT PRIMARY KEY REFERENCES organizations(id) ON DELETE CASCADE,
          settings TEXT NOT NULL DEFAULT '{}',
          updated_by TEXT NOT NULL DEFAULT '', updated_at TEXT NOT NULL
        );
      `);
    },
  },
  {
    version: 8,
    description: 'Diagnostics bundles uploaded by machines',
    up(d) {
      d.exec(`
        CREATE TABLE machine_diagnostics (
          id TEXT PRIMARY KEY,
          org_id TEXT NOT NULL,
          machine_id TEXT NOT NULL REFERENCES machines(id) ON DELETE CASCADE,
          file_name TEXT NOT NULL, size INTEGER NOT NULL, sha256 TEXT NOT NULL,
          summary TEXT NOT NULL DEFAULT '{}', data BLOB NOT NULL, created_at TEXT NOT NULL
        );
        CREATE INDEX machine_diagnostics_machine ON machine_diagnostics(machine_id, created_at);
      `);
    },
  },
  {
    version: 9,
    description: 'Browser sign-in codes for desktop enrollment (loopback + PKCE)',
    up(d) {
      d.exec(`
        CREATE TABLE desktop_connect_codes (
          code_hash TEXT PRIMARY KEY,
          org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          user_email TEXT NOT NULL, code_challenge TEXT NOT NULL, redirect_uri TEXT NOT NULL,
          name TEXT NOT NULL DEFAULT '', tags TEXT NOT NULL DEFAULT '[]',
          created_at TEXT NOT NULL, expires_at TEXT NOT NULL, used_at TEXT
        );
      `);
    },
  },
  {
    version: 10,
    description: 'Managed app settings per organization (updates, proxy, AI)',
    up(d) {
      d.exec(`
        CREATE TABLE managed_settings (
          org_id TEXT PRIMARY KEY REFERENCES organizations(id) ON DELETE CASCADE,
          settings TEXT NOT NULL, version INTEGER NOT NULL,
          updated_by TEXT NOT NULL, updated_at TEXT NOT NULL
        );
      `);
    },
  },
  {
    version: 11,
    description: 'Central credential versions and rotation history',
    up(d) {
      d.exec(`
        ALTER TABLE central_credentials ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
        CREATE TABLE credential_versions (
          credential_id TEXT NOT NULL REFERENCES central_credentials(id) ON DELETE CASCADE,
          version INTEGER NOT NULL, created_by TEXT NOT NULL, created_at TEXT NOT NULL,
          PRIMARY KEY (credential_id, version)
        );
        INSERT INTO credential_versions (credential_id, version, created_by, created_at)
          SELECT id, 1, created_by, created_at FROM central_credentials;
      `);
    },
  },
  {
    version: 12,
    description: 'Outbound webhooks and their delivery queue',
    up(d) {
      d.exec(`
        CREATE TABLE webhooks (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          name TEXT NOT NULL, url TEXT NOT NULL, events TEXT NOT NULL DEFAULT '[]', headers TEXT NOT NULL DEFAULT '{}',
          secret_ciphertext TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, paused_reason TEXT,
          consecutive_failures INTEGER NOT NULL DEFAULT 0, last_success_at TEXT, last_failure_at TEXT,
          created_by TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL, updated_at TEXT NOT NULL
        );
        CREATE INDEX idx_webhooks_org ON webhooks(org_id);
        CREATE TABLE webhook_deliveries (
          id TEXT PRIMARY KEY, webhook_id TEXT NOT NULL REFERENCES webhooks(id) ON DELETE CASCADE,
          org_id TEXT NOT NULL, event TEXT NOT NULL, payload TEXT NOT NULL,
          status TEXT NOT NULL DEFAULT 'pending', attempts INTEGER NOT NULL DEFAULT 0,
          next_attempt_at TEXT, last_status INTEGER, last_error TEXT, duration_ms INTEGER,
          created_at TEXT NOT NULL, delivered_at TEXT
        );
        CREATE INDEX idx_webhook_deliveries_due ON webhook_deliveries(status, next_attempt_at);
        CREATE INDEX idx_webhook_deliveries_hook ON webhook_deliveries(webhook_id, created_at);
      `);
    },
  },
  {
    version: 13,
    description: 'Notification outbox (email, Slack, webhook channels) with retries',
    up(d) {
      d.exec(`
        CREATE TABLE channel_outbox (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL,
          channel_id TEXT NOT NULL REFERENCES notification_channels(id) ON DELETE CASCADE,
          kind TEXT NOT NULL, subject TEXT NOT NULL, message TEXT NOT NULL, context TEXT NOT NULL DEFAULT '',
          status TEXT NOT NULL DEFAULT 'pending', attempts INTEGER NOT NULL DEFAULT 0,
          next_attempt_at TEXT, last_error TEXT, created_at TEXT NOT NULL, sent_at TEXT
        );
        CREATE INDEX idx_channel_outbox_due ON channel_outbox(status, next_attempt_at);
        CREATE INDEX idx_channel_outbox_org ON channel_outbox(org_id, created_at);
      `);
    },
  },
  {
    version: 14,
    description: 'Alert acknowledgement and snooze',
    up(d) {
      d.exec(`
        ALTER TABLE alerts ADD COLUMN acknowledged_at TEXT;
        ALTER TABLE alerts ADD COLUMN acknowledged_by TEXT;
        ALTER TABLE alerts ADD COLUMN snoozed_until TEXT;
      `);
    },
  },
  {
    version: 15,
    description: 'SCIM 2.0 provisioning tokens and resource links',
    up(d) {
      d.exec(`
        CREATE TABLE scim_tokens (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          name TEXT NOT NULL, token_hash TEXT NOT NULL UNIQUE, token_prefix TEXT NOT NULL,
          default_role TEXT NOT NULL DEFAULT 'user', created_by TEXT NOT NULL DEFAULT '',
          created_at TEXT NOT NULL, last_used_at TEXT, revoked_at TEXT
        );
        CREATE TABLE scim_links (
          org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          resource_type TEXT NOT NULL, resource_id TEXT NOT NULL, external_id TEXT,
          created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
          PRIMARY KEY (org_id, resource_type, resource_id)
        );
        CREATE UNIQUE INDEX idx_scim_links_external ON scim_links(org_id, resource_type, external_id) WHERE external_id IS NOT NULL;
      `);
    },
  },
  {
    version: 16,
    description: 'Organization security settings, password age and reset tokens',
    up(d) {
      d.exec(`
        CREATE TABLE org_security (
          org_id TEXT PRIMARY KEY REFERENCES organizations(id) ON DELETE CASCADE,
          settings TEXT NOT NULL DEFAULT '{}', updated_by TEXT NOT NULL DEFAULT '', updated_at TEXT NOT NULL
        );
        ALTER TABLE users ADD COLUMN password_changed_at TEXT;
        UPDATE users SET password_changed_at = created_at WHERE password_hash IS NOT NULL;
        CREATE TABLE password_resets (
          token_hash TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
          purpose TEXT NOT NULL, created_at TEXT NOT NULL, expires_at TEXT NOT NULL, used_at TEXT
        );
      `);
    },
  },
  {
    version: 17,
    description: 'WebAuthn passkeys',
    up(d) {
      d.exec(`
        CREATE TABLE passkeys (
          credential_id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
          name TEXT NOT NULL, public_key TEXT NOT NULL, alg INTEGER NOT NULL, sign_count INTEGER NOT NULL DEFAULT 0,
          transports TEXT NOT NULL DEFAULT '[]', aaguid TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL, last_used_at TEXT
        );
        CREATE INDEX idx_passkeys_user ON passkeys(user_id);
        CREATE TABLE webauthn_challenges (
          id TEXT PRIMARY KEY, user_id TEXT, purpose TEXT NOT NULL, challenge TEXT NOT NULL,
          created_at TEXT NOT NULL, expires_at TEXT NOT NULL, used_at TEXT
        );
      `);
    },
  },
  {
    version: 18,
    description: 'Server-side sessions (idle timeout, revocation)',
    up(d) {
      d.exec(`
        CREATE TABLE sessions (
          id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
          method TEXT NOT NULL DEFAULT '', ip TEXT NOT NULL DEFAULT '', user_agent TEXT NOT NULL DEFAULT '',
          created_at TEXT NOT NULL, last_seen_at TEXT NOT NULL, expires_at TEXT NOT NULL,
          revoked_at TEXT, revoked_reason TEXT
        );
        CREATE INDEX idx_sessions_user ON sessions(user_id, revoked_at);
      `);
    },
  },
  {
    version: 19,
    description: 'Step-up re-authentication time on sessions',
    up(d) {
      d.exec('ALTER TABLE sessions ADD COLUMN verified_at TEXT');
    },
  },
  {
    version: 20,
    description: 'Personal API tokens (scoped, expiring)',
    up(d) {
      d.exec(`
        CREATE TABLE api_tokens (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
          name TEXT NOT NULL, token_hash TEXT NOT NULL UNIQUE, prefix TEXT NOT NULL,
          permissions TEXT NOT NULL DEFAULT '[]', created_at TEXT NOT NULL, expires_at TEXT NOT NULL,
          last_used_at TEXT, last_used_ip TEXT, revoked_at TEXT
        );
        CREATE INDEX idx_api_tokens_user ON api_tokens(user_id);
        CREATE INDEX idx_api_tokens_org ON api_tokens(org_id);
      `);
    },
  },
  {
    version: 21,
    description: 'Membership expiry; removed members kept for restore',
    up(d) {
      d.exec(`
        ALTER TABLE org_memberships ADD COLUMN expires_at TEXT;
        CREATE TABLE removed_members (
          user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
          org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          role TEXT NOT NULL, group_ids TEXT NOT NULL DEFAULT '[]', reason TEXT NOT NULL DEFAULT '',
          removed_by TEXT NOT NULL DEFAULT '', removed_at TEXT NOT NULL, purge_after TEXT NOT NULL,
          account_disabled INTEGER NOT NULL DEFAULT 0, reassigned_to TEXT,
          PRIMARY KEY (user_id, org_id)
        );
      `);
    },
  },
  {
    version: 22,
    description: 'User time zone preference',
    up(d) {
      d.exec('ALTER TABLE users ADD COLUMN time_zone TEXT');
    },
  },
  {
    version: 23,
    description: 'Fleet policy inheritance, locks and child-organization opt-outs',
    up(d) {
      d.exec(`
        ALTER TABLE fleet_policies ADD COLUMN inherit INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE fleet_policies ADD COLUMN locked INTEGER NOT NULL DEFAULT 0;
        CREATE TABLE policy_opt_outs (
          org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          policy_id TEXT NOT NULL REFERENCES fleet_policies(id) ON DELETE CASCADE,
          opted_out_by TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL,
          PRIMARY KEY (org_id, policy_id)
        );
      `);
    },
  },
  {
    version: 24,
    description: 'SIEM forwarding destinations with a per-destination audit cursor',
    up(d) {
      d.exec(`
        CREATE TABLE siem_destinations (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          name TEXT NOT NULL, kind TEXT NOT NULL, config TEXT NOT NULL DEFAULT '{}', secret_ciphertext TEXT,
          include_children INTEGER NOT NULL DEFAULT 0, enabled INTEGER NOT NULL DEFAULT 1,
          cursor INTEGER NOT NULL DEFAULT 0, forwarded_total INTEGER NOT NULL DEFAULT 0,
          failures INTEGER NOT NULL DEFAULT 0, next_attempt_at TEXT, last_error TEXT, last_attempt_at TEXT, last_success_at TEXT,
          created_by TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL, updated_at TEXT NOT NULL
        );
        CREATE INDEX idx_siem_org ON siem_destinations(org_id);
      `);
    },
  },
  {
    version: 25,
    description: 'AI transcripts (their hashes are on the audit chain)',
    up(d) {
      d.exec(`
        CREATE TABLE ai_transcripts (
          report_id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          model TEXT NOT NULL, system_prompt TEXT NOT NULL, request TEXT NOT NULL, response TEXT NOT NULL,
          sha256 TEXT NOT NULL, audit_id TEXT NOT NULL, created_at TEXT NOT NULL
        );
      `);
    },
  },
  {
    version: 26,
    description: 'API request log and SSO sign-in diagnostics',
    up(d) {
      d.exec(`
        CREATE TABLE api_requests (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL DEFAULT '', at TEXT NOT NULL,
          method TEXT NOT NULL, path TEXT NOT NULL, status INTEGER NOT NULL, outcome TEXT NOT NULL DEFAULT '',
          auth TEXT NOT NULL, token_id TEXT, user_id TEXT, user_email TEXT NOT NULL DEFAULT '',
          ip TEXT NOT NULL DEFAULT '', user_agent TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX idx_api_requests_org ON api_requests(org_id, at);
        CREATE TABLE sso_attempts (
          id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, org_id TEXT NOT NULL, kind TEXT NOT NULL,
          at TEXT NOT NULL, ok INTEGER NOT NULL, stage TEXT NOT NULL, error TEXT,
          steps TEXT NOT NULL DEFAULT '[]', email TEXT NOT NULL DEFAULT '', user_id TEXT, ip TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX idx_sso_attempts_provider ON sso_attempts(provider_id, at);
      `);
    },
  },
  {
    version: 27,
    description: 'Report schedules and report runs',
    up(d) {
      d.exec(`
        CREATE TABLE report_schedules (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          name TEXT NOT NULL, report_id TEXT NOT NULL, days INTEGER NOT NULL, format TEXT NOT NULL DEFAULT 'csv',
          frequency TEXT NOT NULL, hour INTEGER NOT NULL, weekday INTEGER, day_of_month INTEGER, time_zone TEXT NOT NULL,
          channel_ids TEXT NOT NULL DEFAULT '[]', enabled INTEGER NOT NULL DEFAULT 1, next_run_at TEXT,
          created_by TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL, updated_at TEXT NOT NULL
        );
        CREATE INDEX idx_report_schedules_due ON report_schedules(enabled, next_run_at);
        CREATE TABLE report_runs (
          id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
          report_id TEXT NOT NULL, schedule_id TEXT, days INTEGER NOT NULL, status TEXT NOT NULL,
          row_count INTEGER NOT NULL DEFAULT 0, summary TEXT NOT NULL DEFAULT '', result TEXT, error TEXT,
          delivery TEXT NOT NULL DEFAULT '[]', requested_by TEXT NOT NULL DEFAULT '',
          started_at TEXT NOT NULL, finished_at TEXT
        );
        CREATE INDEX idx_report_runs_org ON report_runs(org_id, started_at);
      `);
    },
  },
];

export const LATEST_SCHEMA = MIGRATIONS[MIGRATIONS.length - 1].version;
