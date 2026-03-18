/**
 * Permission checking utilities for RBAC (T-049).
 */

import { type Role, type Permission, ROLE_PERMISSIONS, ROLE_HIERARCHY } from '../types/auth';

/** Check if a role has a specific permission */
export function hasPermission(role: Role, permission: Permission): boolean {
  return ROLE_PERMISSIONS[role]?.includes(permission) ?? false;
}

/** Check if a role has any of the given permissions */
export function hasAnyPermission(role: Role, permissions: Permission[]): boolean {
  return permissions.some(p => hasPermission(role, p));
}

/** Check if a role has all of the given permissions */
export function hasAllPermissions(role: Role, permissions: Permission[]): boolean {
  return permissions.every(p => hasPermission(role, p));
}

/** Check if role A outranks role B in hierarchy */
export function outranks(roleA: Role, roleB: Role): boolean {
  return (ROLE_HIERARCHY[roleA] ?? 0) > (ROLE_HIERARCHY[roleB] ?? 0);
}

/** Check if a role can manage another role */
export function canManageRole(managerRole: Role, targetRole: Role): boolean {
  // Can only manage roles below you in hierarchy
  return outranks(managerRole, targetRole);
}

/** Check if a role can access the admin console */
export function canAccessAdmin(role: Role): boolean {
  return hasPermission(role, 'admin.access');
}

/** Get all permissions for a role */
export function getPermissions(role: Role): Permission[] {
  return ROLE_PERMISSIONS[role] ?? [];
}

/** Get roles that a manager can assign */
export function getAssignableRoles(managerRole: Role): Role[] {
  const managerLevel = ROLE_HIERARCHY[managerRole] ?? 0;
  return (Object.entries(ROLE_HIERARCHY) as [Role, number][])
    .filter(([_, level]) => level < managerLevel)
    .map(([role]) => role);
}
