/**
 * Shared workspace types (T-053).
 * Shared connections, sync templates, and automation templates.
 */

export interface SharedConnection {
  id: string;
  name: string;
  connectorType: string;
  /** Connection configuration (credentials stored separately) */
  config: Record<string, unknown>;
  /** Who owns this shared connection */
  ownerId: string;
  ownerName: string;
  /** Organization it belongs to */
  organizationId: string;
  /** Members who have access */
  sharedWith: SharedMember[];
  /** When it was last propagated to clients */
  lastPropagatedAt?: string;
  /** Propagation target count */
  propagationStatus: PropagationStatus;
  createdAt: string;
  updatedAt: string;
  isActive: boolean;
}

export interface SharedMember {
  userId: string;
  userName: string;
  userEmail: string;
  accessLevel: 'read' | 'write' | 'admin';
  addedAt: string;
  addedBy: string;
}

export interface PropagationStatus {
  targetCount: number;
  confirmedCount: number;
  failedCount: number;
  status: 'pending' | 'propagating' | 'complete' | 'partial' | 'failed';
  lastAttemptAt?: string;
}

export interface SyncTemplate {
  id: string;
  name: string;
  description: string;
  /** Source and destination configuration */
  sourceConnector: string;
  sourcePath: string;
  destConnector: string;
  destPath: string;
  /** Sync options */
  syncMode: 'mirror' | 'merge' | 'one_way';
  conflictResolution: 'newest_wins' | 'oldest_wins' | 'manual' | 'duplicate';
  fileFilters: string[];
  excludePatterns: string[];
  /** Schedule (cron expression) */
  schedule?: string;
  /** Sharing */
  organizationId: string;
  ownerId: string;
  ownerName: string;
  sharedWith: SharedMember[];
  /** Activation tracking */
  activatedByUsers: string[];
  propagationStatus: PropagationStatus;
  createdAt: string;
  updatedAt: string;
  isActive: boolean;
}

export interface AutomationTemplate {
  id: string;
  name: string;
  description: string;
  /** Trigger conditions */
  trigger: AutomationTrigger;
  /** Actions to perform */
  actions: AutomationAction[];
  /** Sharing */
  organizationId: string;
  ownerId: string;
  ownerName: string;
  sharedWith: SharedMember[];
  activatedByUsers: string[];
  propagationStatus: PropagationStatus;
  createdAt: string;
  updatedAt: string;
  isActive: boolean;
}

export interface AutomationTrigger {
  type: 'file_created' | 'file_modified' | 'schedule' | 'manual' | 'webhook';
  config: Record<string, unknown>;
}

export interface AutomationAction {
  type: 'move' | 'copy' | 'rename' | 'tag' | 'notify' | 'sync' | 'transform';
  config: Record<string, unknown>;
  order: number;
}

export interface Workspace {
  id: string;
  name: string;
  description: string;
  organizationId: string;
  connections: SharedConnection[];
  syncTemplates: SyncTemplate[];
  automationTemplates: AutomationTemplate[];
  members: SharedMember[];
  createdAt: string;
  updatedAt: string;
}
