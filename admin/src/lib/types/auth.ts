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
  | 'sso.configure';

/** Default permissions per role */
export const ROLE_PERMISSIONS: Record<Role, Permission[]> = {
  org_owner: [
    'admin.access', 'users.read', 'users.write', 'users.invite', 'users.deactivate', 'users.remove',
    'roles.read', 'roles.write', 'devices.read', 'devices.write',
    'policies.read', 'policies.write', 'approvals.read', 'approvals.approve', 'approvals.deny',
    'audit.read', 'audit.export', 'connectors.read', 'connectors.write',
    'workspaces.read', 'workspaces.write', 'billing.read', 'billing.write',
    'ai_governance.read', 'ai_governance.write', 'sso.configure',
  ],
  org_admin: [
    'admin.access', 'users.read', 'users.write', 'users.invite', 'users.deactivate',
    'roles.read', 'roles.write', 'devices.read', 'devices.write',
    'policies.read', 'policies.write', 'approvals.read', 'approvals.approve', 'approvals.deny',
    'audit.read', 'audit.export', 'connectors.read', 'connectors.write',
    'workspaces.read', 'workspaces.write', 'billing.read',
    'ai_governance.read', 'ai_governance.write',
  ],
  security_admin: [
    'admin.access', 'users.read', 'devices.read',
    'policies.read', 'policies.write', 'approvals.read', 'approvals.approve', 'approvals.deny',
    'audit.read', 'audit.export', 'connectors.read',
    'ai_governance.read', 'ai_governance.write', 'sso.configure',
  ],
  approver: [
    'admin.access', 'approvals.read', 'approvals.approve', 'approvals.deny',
    'audit.read',
  ],
  operator: [
    'admin.access', 'users.read', 'devices.read', 'devices.write',
    'connectors.read', 'connectors.write', 'workspaces.read', 'workspaces.write',
    'audit.read',
  ],
  user: [],
  read_only_auditor: [
    'admin.access', 'users.read', 'devices.read',
    'policies.read', 'approvals.read', 'audit.read', 'audit.export',
    'connectors.read', 'workspaces.read',
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
