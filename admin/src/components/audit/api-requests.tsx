'use client';

/** Audit → API requests: what scripts did with API tokens, and refused requests. */
import React, { useState } from 'react';
import { Select } from '@/components/ui/select';
import { Badge } from '@/components/ui/badge';
import { DataTable, useServerTable, type Column } from '@/components/ui/data-table';
import { ExportsPanel } from '@/components/exports/exports-panel';
import { formatDateTime } from '@/lib/utils/format';
import type { ApiRequest } from '@/lib/db/repos/api-log';

const COLUMNS: Column<ApiRequest>[] = [
  { key: 'at', label: 'Time', required: true, render: (r) => <span className="whitespace-nowrap text-xs">{formatDateTime(r.at)}</span> },
  {
    key: 'request',
    label: 'Request',
    required: true,
    render: (r) => (
      <span className="font-mono text-xs">
        {r.method} {r.path}
      </span>
    ),
  },
  {
    key: 'result',
    label: 'Result',
    render: (r) => (
      <>
        <Badge variant={r.status < 400 ? 'success' : 'error'}>{r.status}</Badge> <span className="text-xs">{r.outcome}</span>
      </>
    ),
  },
  {
    key: 'who',
    label: 'Who',
    render: (r) => (
      <span className="text-xs">
        {r.userEmail || '—'} <span className="text-foreground-tertiary">· {r.auth === 'token' ? `token ${r.tokenId ?? 'unknown'}` : 'session'}</span>
      </span>
    ),
  },
  { key: 'ip', label: 'Address', render: (r) => <span className="text-xs">{r.ip}</span> },
  { key: 'agent', label: 'Client', defaultHidden: true, render: (r) => <span className="text-xs text-foreground-tertiary">{r.userAgent}</span> },
];

export function ApiRequestsLog() {
  const [result, setResult] = useState('');
  const t = useServerTable<ApiRequest>('api-requests', '/api/api-requests', 'requests', { result });
  return (
    <div>
      <div className="mb-3 flex items-center gap-3">
        <Select
          options={[
            { value: '', label: 'All requests' },
            { value: 'ok', label: 'Accepted' },
            { value: 'refused', label: 'Refused' },
          ]}
          value={result}
          onChange={(e) => setResult(e.target.value)}
          className="w-48"
        />
        <span className="text-xs text-foreground-tertiary">Requests made with API tokens, and requests refused by the network rule. Kept {String(t.data?.keepDays ?? 30)} days.</span>
      </div>
      <details className="mb-3">
        <summary className="cursor-pointer text-sm">Export all matching requests</summary>
        <div className="mt-2">
          <ExportsPanel kind="api_requests" formats={['csv', 'json']} params={result ? { result } : {}} />
        </div>
      </details>
      <DataTable
        id="api-requests"
        columns={COLUMNS}
        rows={t.rows}
        rowKey={(r) => r.id}
        loading={t.loading}
        error={t.error}
        empty="No API requests yet"
        pagination={t.pagination}
        limit={t.limit}
        onLimit={t.setLimit}
        onOffset={t.setOffset}
      />
    </div>
  );
}
