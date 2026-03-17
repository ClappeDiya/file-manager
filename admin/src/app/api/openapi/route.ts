import { NextResponse } from 'next/server';

/**
 * OpenAPI 3.0 Specification (T-059)
 *
 * GET /api/openapi - Returns the auto-generated OpenAPI 3.0 spec
 */

const openApiSpec = {
  openapi: '3.0.3',
  info: {
    title: 'UFOP Admin API',
    description: 'Unified File Operations Platform - REST API for administration, transfers, sync, and governance.',
    version: '0.1.0',
    contact: {
      name: 'UFOP Team',
    },
  },
  servers: [
    {
      url: '/api',
      description: 'Current server',
    },
  ],
  security: [
    { BearerAuth: [] },
    { ApiKeyAuth: [] },
  ],
  components: {
    securitySchemes: {
      BearerAuth: {
        type: 'http',
        scheme: 'bearer',
        bearerFormat: 'JWT',
        description: 'OAuth 2.0 client credentials (Enterprise) or API token',
      },
      ApiKeyAuth: {
        type: 'apiKey',
        in: 'header',
        name: 'X-API-Key',
        description: 'API key (Business tier)',
      },
    },
    schemas: {
      Error: {
        type: 'object',
        properties: {
          error: { type: 'string' },
          code: { type: 'string' },
        },
      },
      Transfer: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          source: { type: 'string' },
          destination: { type: 'string' },
          status: { type: 'string', enum: ['queued', 'active', 'paused', 'completed', 'failed', 'cancelled'] },
          progress: { type: 'number' },
          total_bytes: { type: 'integer', format: 'int64' },
          transferred_bytes: { type: 'integer', format: 'int64' },
          speed_bps: { type: 'integer', format: 'int64' },
          priority: { type: 'string', enum: ['high', 'normal', 'low'] },
          created_at: { type: 'string', format: 'date-time' },
          updated_at: { type: 'string', format: 'date-time' },
          error_message: { type: 'string', nullable: true },
          verify_checksum: { type: 'boolean' },
        },
      },
      SyncPair: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          name: { type: 'string' },
          source: { type: 'string' },
          destination: { type: 'string' },
          direction: { type: 'string', enum: ['bidirectional', 'source-to-dest', 'dest-to-source'] },
          schedule: { type: 'string', nullable: true },
          status: { type: 'string', enum: ['idle', 'syncing', 'paused', 'error'] },
          last_run: { type: 'string', format: 'date-time', nullable: true },
          files_synced: { type: 'integer' },
          bytes_synced: { type: 'integer', format: 'int64' },
        },
      },
      Webhook: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          url: { type: 'string', format: 'uri' },
          events: { type: 'array', items: { type: 'string' } },
          is_active: { type: 'boolean' },
          created_at: { type: 'string', format: 'date-time' },
          last_triggered: { type: 'string', format: 'date-time', nullable: true },
          failure_count: { type: 'integer' },
        },
      },
      User: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          email: { type: 'string', format: 'email' },
          name: { type: 'string' },
          role: { type: 'string', enum: ['super_admin', 'org_admin', 'manager', 'user', 'viewer'] },
          isActive: { type: 'boolean' },
          createdAt: { type: 'string', format: 'date-time' },
        },
      },
      Policy: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          name: { type: 'string' },
          type: { type: 'string' },
          rules: { type: 'array', items: { type: 'object' } },
          enabled: { type: 'boolean' },
        },
      },
      Connector: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          name: { type: 'string' },
          protocol: { type: 'string', enum: ['sftp', 'ftp', 'webdav', 's3', 'gcs', 'azure', 'smb', 'nfs'] },
          host: { type: 'string' },
          port: { type: 'integer' },
          status: { type: 'string', enum: ['connected', 'disconnected', 'error'] },
        },
      },
      AiGovernanceSettings: {
        type: 'object',
        properties: {
          ai_enabled: { type: 'boolean' },
          auto_suggestions: { type: 'boolean' },
          natural_language_commands: { type: 'boolean' },
          require_confirmation_for_destructive: { type: 'boolean' },
          max_files_per_action: { type: 'integer' },
          allowed_actions: { type: 'array', items: { type: 'string' } },
          audit_retention_days: { type: 'integer' },
        },
      },
    },
  },
  paths: {
    '/health': {
      get: {
        summary: 'Health check',
        tags: ['System'],
        responses: {
          '200': { description: 'Server is healthy' },
        },
      },
    },
    '/users': {
      get: {
        summary: 'List users',
        tags: ['Users'],
        parameters: [
          { name: 'role', in: 'query', schema: { type: 'string' } },
          { name: 'search', in: 'query', schema: { type: 'string' } },
          { name: 'active', in: 'query', schema: { type: 'boolean' } },
        ],
        responses: {
          '200': { description: 'List of users' },
          '401': { description: 'Unauthorized' },
        },
      },
      post: {
        summary: 'Create user',
        tags: ['Users'],
        requestBody: {
          content: {
            'application/json': {
              schema: {
                type: 'object',
                required: ['email'],
                properties: {
                  email: { type: 'string', format: 'email' },
                  name: { type: 'string' },
                  role: { type: 'string' },
                },
              },
            },
          },
        },
        responses: {
          '201': { description: 'User created' },
          '400': { description: 'Invalid request' },
          '409': { description: 'User already exists' },
        },
      },
    },
    '/roles': {
      get: { summary: 'List roles', tags: ['Roles'], responses: { '200': { description: 'List of roles' } } },
    },
    '/devices': {
      get: { summary: 'List devices', tags: ['Devices'], responses: { '200': { description: 'List of devices' } } },
    },
    '/policies': {
      get: { summary: 'List policies', tags: ['Policies'], responses: { '200': { description: 'List of policies' } } },
      post: { summary: 'Create policy', tags: ['Policies'], responses: { '201': { description: 'Policy created' } } },
    },
    '/approvals': {
      get: { summary: 'List approval requests', tags: ['Approvals'], responses: { '200': { description: 'List of approvals' } } },
      post: { summary: 'Create approval request', tags: ['Approvals'], responses: { '201': { description: 'Approval created' } } },
    },
    '/audit': {
      get: {
        summary: 'Query audit log',
        tags: ['Audit'],
        parameters: [
          { name: 'action', in: 'query', schema: { type: 'string' } },
          { name: 'user', in: 'query', schema: { type: 'string' } },
          { name: 'from', in: 'query', schema: { type: 'string', format: 'date-time' } },
          { name: 'to', in: 'query', schema: { type: 'string', format: 'date-time' } },
          { name: 'limit', in: 'query', schema: { type: 'integer', default: 50 } },
          { name: 'offset', in: 'query', schema: { type: 'integer', default: 0 } },
        ],
        responses: { '200': { description: 'Audit log entries' } },
      },
    },
    '/connectors': {
      get: { summary: 'List connectors', tags: ['Connectors'], responses: { '200': { description: 'List of connectors' } } },
      post: { summary: 'Create connector', tags: ['Connectors'], responses: { '201': { description: 'Connector created' } } },
    },
    '/connectors/{id}': {
      get: { summary: 'Get connector', tags: ['Connectors'], parameters: [{ name: 'id', in: 'path', required: true, schema: { type: 'string' } }], responses: { '200': { description: 'Connector details' } } },
      patch: { summary: 'Update connector', tags: ['Connectors'], parameters: [{ name: 'id', in: 'path', required: true, schema: { type: 'string' } }], responses: { '200': { description: 'Connector updated' } } },
      delete: { summary: 'Delete connector', tags: ['Connectors'], parameters: [{ name: 'id', in: 'path', required: true, schema: { type: 'string' } }], responses: { '200': { description: 'Connector deleted' } } },
      post: { summary: 'Connector action (test/connect/disconnect)', tags: ['Connectors'], parameters: [{ name: 'id', in: 'path', required: true, schema: { type: 'string' } }], responses: { '200': { description: 'Action result' } } },
    },
    '/transfers': {
      get: {
        summary: 'List transfers',
        tags: ['Transfers'],
        parameters: [
          { name: 'status', in: 'query', schema: { type: 'string' } },
          { name: 'priority', in: 'query', schema: { type: 'string' } },
          { name: 'limit', in: 'query', schema: { type: 'integer', default: 50 } },
          { name: 'offset', in: 'query', schema: { type: 'integer', default: 0 } },
        ],
        responses: { '200': { description: 'List of transfers' } },
      },
      post: {
        summary: 'Trigger transfer',
        tags: ['Transfers'],
        requestBody: {
          content: {
            'application/json': {
              schema: {
                type: 'object',
                required: ['source', 'destination'],
                properties: {
                  source: { type: 'string' },
                  destination: { type: 'string' },
                  priority: { type: 'string', enum: ['high', 'normal', 'low'] },
                  verify_checksum: { type: 'boolean' },
                },
              },
            },
          },
        },
        responses: {
          '201': { description: 'Transfer created' },
          '400': { description: 'Invalid request' },
          '429': { description: 'Rate limited' },
        },
      },
    },
    '/transfers/{id}': {
      get: { summary: 'Get transfer', tags: ['Transfers'], parameters: [{ name: 'id', in: 'path', required: true, schema: { type: 'string' } }], responses: { '200': { description: 'Transfer details' } } },
      patch: { summary: 'Update transfer (pause/resume/cancel)', tags: ['Transfers'], parameters: [{ name: 'id', in: 'path', required: true, schema: { type: 'string' } }], responses: { '200': { description: 'Transfer updated' } } },
      delete: { summary: 'Cancel and remove transfer', tags: ['Transfers'], parameters: [{ name: 'id', in: 'path', required: true, schema: { type: 'string' } }], responses: { '200': { description: 'Transfer deleted' } } },
    },
    '/sync': {
      get: { summary: 'List sync pairs', tags: ['Sync'], responses: { '200': { description: 'List of sync pairs' } } },
      post: {
        summary: 'Create sync pair',
        tags: ['Sync'],
        requestBody: {
          content: {
            'application/json': {
              schema: {
                type: 'object',
                required: ['name', 'source', 'destination'],
                properties: {
                  name: { type: 'string' },
                  source: { type: 'string' },
                  destination: { type: 'string' },
                  direction: { type: 'string' },
                  schedule: { type: 'string' },
                },
              },
            },
          },
        },
        responses: { '201': { description: 'Sync pair created' } },
      },
    },
    '/sync/{id}': {
      get: { summary: 'Get sync pair', tags: ['Sync'], parameters: [{ name: 'id', in: 'path', required: true, schema: { type: 'string' } }], responses: { '200': { description: 'Sync pair details' } } },
      patch: { summary: 'Update sync pair or trigger run', tags: ['Sync'], parameters: [{ name: 'id', in: 'path', required: true, schema: { type: 'string' } }], responses: { '200': { description: 'Sync pair updated' } } },
      delete: { summary: 'Delete sync pair', tags: ['Sync'], parameters: [{ name: 'id', in: 'path', required: true, schema: { type: 'string' } }], responses: { '200': { description: 'Sync pair deleted' } } },
    },
    '/webhooks': {
      get: { summary: 'List webhooks', tags: ['Webhooks'], responses: { '200': { description: 'List of webhooks' } } },
      post: {
        summary: 'Register webhook',
        tags: ['Webhooks'],
        requestBody: {
          content: {
            'application/json': {
              schema: {
                type: 'object',
                required: ['url', 'events'],
                properties: {
                  url: { type: 'string', format: 'uri' },
                  events: { type: 'array', items: { type: 'string' } },
                },
              },
            },
          },
        },
        responses: { '201': { description: 'Webhook created' } },
      },
    },
    '/webhooks/{id}': {
      get: { summary: 'Get webhook', tags: ['Webhooks'], parameters: [{ name: 'id', in: 'path', required: true, schema: { type: 'string' } }], responses: { '200': { description: 'Webhook details' } } },
      patch: { summary: 'Update webhook', tags: ['Webhooks'], parameters: [{ name: 'id', in: 'path', required: true, schema: { type: 'string' } }], responses: { '200': { description: 'Webhook updated' } } },
      delete: { summary: 'Delete webhook', tags: ['Webhooks'], parameters: [{ name: 'id', in: 'path', required: true, schema: { type: 'string' } }], responses: { '200': { description: 'Webhook deleted' } } },
    },
    '/ai-governance': {
      get: {
        summary: 'Get AI governance settings',
        tags: ['AI Governance'],
        parameters: [
          { name: 'view', in: 'query', schema: { type: 'string', enum: ['settings', 'audit', 'toggles'] } },
        ],
        responses: { '200': { description: 'AI governance settings or audit log' } },
      },
      post: { summary: 'Update AI governance settings', tags: ['AI Governance'], responses: { '200': { description: 'Settings updated' } } },
    },
    '/openapi': {
      get: { summary: 'OpenAPI 3.0 specification', tags: ['System'], security: [], responses: { '200': { description: 'OpenAPI spec' } } },
    },
  },
  tags: [
    { name: 'System', description: 'Health and status endpoints' },
    { name: 'Users', description: 'User management' },
    { name: 'Roles', description: 'Role management' },
    { name: 'Devices', description: 'Device management' },
    { name: 'Policies', description: 'Policy management' },
    { name: 'Approvals', description: 'Approval workflow' },
    { name: 'Audit', description: 'Audit log' },
    { name: 'Connectors', description: 'Protocol connectors' },
    { name: 'Transfers', description: 'File transfer operations' },
    { name: 'Sync', description: 'Sync pair management' },
    { name: 'Webhooks', description: 'Webhook event subscriptions' },
    { name: 'AI Governance', description: 'AI feature governance and audit' },
  ],
};

/** GET /api/openapi - OpenAPI 3.0 spec */
export async function GET() {
  return NextResponse.json(openApiSpec, {
    headers: {
      'Content-Type': 'application/json',
      'Access-Control-Allow-Origin': '*',
    },
  });
}
