/**
 * Authentication and authorization types for the admin console.
 */

/** All available roles in the system (T-049) */
export type Role =
  | 'org_owner'
  | 'org_admin'
  | 'security_admin'
  | 'approver'
  | 'operator'
  | 'user'
  | 'read_only_auditor';

export const ROLE_LABELS: Record<Role, string> = {
  org_owner: 'Organization Owner',
  org_admin: 'Organization Admin',
  security_admin: 'Security Admin',
  approver: 'Approver',
  operator: 'Operator',
  user: 'User',
  read_only_auditor: 'Read-Only Auditor',
};

export const ROLE_HIERARCHY: Record<Role, number> = {
  org_owner: 100,
  org_admin: 90,
  security_admin: 80,
  approver: 70,
  operator: 60,
  user: 40,
  read_only_auditor: 10,
};

/** Permissions that can be assigned to roles */
export type Permission =
  | 'admin.access'
  | 'users.read'
  | 'users.write'
  | 'users.invite'
  | 'users.deactivate'
  | 'users.remove'
  | 'roles.read'
  | 'roles.write'
  | 'devices.read'
  | 'devices.write'
  | 'policies.read'
  | 'policies.write'
  | 'approvals.read'
  | 'approvals.approve'
  | 'approvals.deny'
  | 'audit.read'
  | 'audit.export'
  | 'connectors.read'
  | 'connectors.write'
  | 'workspaces.read'
  | 'workspaces.write'
  | 'billing.read'
  | 'billing.write'
  | 'ai_governance.read'
  | 'ai_governance.write'
  | 'sso.configure'
  | 'orgs.read'
  | 'orgs.write'
  | 'groups.read'
  | 'groups.write'
  | 'fleet.read'
  | 'fleet.write'
  | 'fleet.enroll'
  | 'alerts.read'
  | 'alerts.write'
  | 'credentials.write'
  | 'remote.execute'
  | 'ai.read'
  | 'ai.write';

/** Fleet / tenancy permissions added for the backup console. */
const FLEET_ALL: Permission[] = ['orgs.read', 'orgs.write', 'groups.read', 'groups.write', 'fleet.read', 'fleet.write', 'fleet.enroll', 'alerts.read', 'alerts.write', 'credentials.write', 'remote.execute', 'ai.read', 'ai.write'];

/** Default permissions per role */
export const ROLE_PERMISSIONS: Record<Role, Permission[]> = {
  org_owner: [
    'admin.access', 'users.read', 'users.write', 'users.invite', 'users.deactivate', 'users.remove',
    'roles.read', 'roles.write', 'devices.read', 'devices.write',
    'policies.read', 'policies.write', 'approvals.read', 'approvals.approve', 'approvals.deny',
    'audit.read', 'audit.export', 'connectors.read', 'connectors.write',
    'workspaces.read', 'workspaces.write', 'billing.read', 'billing.write',
    'ai_governance.read', 'ai_governance.write', 'sso.configure',
    ...FLEET_ALL,
  ],
  org_admin: [
    'admin.access', 'users.read', 'users.write', 'users.invite', 'users.deactivate',
    'roles.read', 'roles.write', 'devices.read', 'devices.write',
    'policies.read', 'policies.write', 'approvals.read', 'approvals.approve', 'approvals.deny',
    'audit.read', 'audit.export', 'connectors.read', 'connectors.write',
    'workspaces.read', 'workspaces.write', 'billing.read',
    'ai_governance.read', 'ai_governance.write',
    ...FLEET_ALL,
  ],
  security_admin: [
    'admin.access', 'users.read', 'devices.read',
    'policies.read', 'policies.write', 'approvals.read', 'approvals.approve', 'approvals.deny',
    'audit.read', 'audit.export', 'connectors.read',
    'ai_governance.read', 'ai_governance.write', 'sso.configure',
    'orgs.read', 'groups.read', 'fleet.read', 'alerts.read', 'alerts.write', 'credentials.write', 'ai.read',
  ],
  approver: [
    'admin.access', 'approvals.read', 'approvals.approve', 'approvals.deny',
    'audit.read', 'fleet.read',
  ],
  operator: [
    'admin.access', 'users.read', 'devices.read', 'devices.write',
    'connectors.read', 'connectors.write', 'workspaces.read', 'workspaces.write',
    'audit.read',
    'orgs.read', 'groups.read', 'fleet.read', 'fleet.write', 'fleet.enroll', 'alerts.read', 'remote.execute', 'ai.read',
  ],
  // Machine owners: see only machines their groups grant (access scoping).
  user: ['admin.access', 'fleet.read', 'groups.read'],
  read_only_auditor: [
    'admin.access', 'users.read', 'devices.read',
    'policies.read', 'approvals.read', 'audit.read', 'audit.export',
    'connectors.read', 'workspaces.read',
    'orgs.read', 'groups.read', 'fleet.read', 'alerts.read', 'ai.read',
  ],
};

export interface AdminUser {
  id: string;
  email: string;
  name: string;
  role: Role;
  permissions: Permission[];
  organizationId: string;
  avatarUrl?: string;
  ssoProvider?: 'saml' | 'scim' | null;
  lastLoginAt?: string;
  createdAt: string;
  isActive: boolean;
  /** IANA time zone for showing times (null = the browser's). */
  timeZone?: string | null;
}

export interface SSOConfig {
  provider: 'saml' | 'scim';
  entityId: string;
  ssoUrl: string;
  certificate: string;
  enabled: boolean;
  /** SCIM-only fields */
  scimEndpoint?: string;
  scimToken?: string;
}

export interface AuthSession {
  user: AdminUser;
  token: string;
  expiresAt: string;
}
