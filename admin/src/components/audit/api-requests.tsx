'use client';

/** Audit → API requests: what scripts did with API tokens, and refused requests. */
import React, { useState } from 'react';
import { Card } from '@/components/ui/card';
import { Select } from '@/components/ui/select';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { ErrorBanner, Loading } from '@/components/fleet/badges';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { useApi } from '@/lib/use-api';
import { formatDateTime } from '@/lib/utils/format';
import type { ApiRequest } from '@/lib/db/repos/api-log';
import { ExportsPanel } from '@/components/exports/exports-panel';

const PAGE = 50;

export function ApiRequestsLog() {
  const [result, setResult] = useState('');
  const [offset, setOffset] = useState(0);
  const { data, error, loading } = useApi<{ requests: ApiRequest[]; total: number; keepDays: number }>(`/api/api-requests?limit=${PAGE}&offset=${offset}${result ? `&result=${result}` : ''}`);
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
          onChange={(e) => {
            setResult(e.target.value);
            setOffset(0);
          }}
          className="w-48"
        />
        <span className="text-xs text-foreground-tertiary">Requests made with API tokens, and requests refused by the network rule. Kept {data?.keepDays ?? 30} days.</span>
      </div>
      <details className="mb-3">
        <summary className="cursor-pointer text-sm">Export all matching requests</summary>
        <div className="mt-2">
          <ExportsPanel kind="api_requests" formats={['csv', 'json']} params={result ? { result } : {}} />
        </div>
      </details>
      <ErrorBanner message={error} />
      {loading && !data ? (
        <Loading />
      ) : (
        <Card padding="none">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Time</TableHead>
                <TableHead>Request</TableHead>
                <TableHead>Result</TableHead>
                <TableHead className="hidden md:table-cell">Who</TableHead>
                <TableHead className="hidden lg:table-cell">From</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data?.requests.map((r) => (
                <TableRow key={r.id}>
                  <TableCell className="whitespace-nowrap text-xs">{formatDateTime(r.at)}</TableCell>
                  <TableCell className="font-mono text-xs">
                    {r.method} {r.path}
                  </TableCell>
                  <TableCell>
                    <Badge variant={r.status < 400 ? 'success' : 'error'}>{r.status}</Badge> <span className="text-xs">{r.outcome}</span>
                  </TableCell>
                  <TableCell className="hidden text-xs md:table-cell">
                    {r.userEmail || '—'} {r.auth === 'token' ? <span className="text-foreground-tertiary">· token {r.tokenId ?? 'unknown'}</span> : <span className="text-foreground-tertiary">· session</span>}
                  </TableCell>
                  <TableCell className="hidden text-xs lg:table-cell">
                    {r.ip} <span className="text-foreground-tertiary">{r.userAgent.slice(0, 40)}</span>
                  </TableCell>
                </TableRow>
              ))}
              {data && !data.requests.length && (
                <TableRow>
                  <TableCell colSpan={5} className="py-8 text-center text-foreground-secondary">
                    No API requests yet
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </Card>
      )}
      {data && data.total > PAGE && (
        <div className="mt-3 flex items-center gap-2 text-sm">
          <Button size="sm" variant="outline" disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - PAGE))}>
            Newer
          </Button>
          <span>
            {offset + 1}–{Math.min(offset + PAGE, data.total)} of {data.total}
          </span>
          <Button size="sm" variant="outline" disabled={offset + PAGE >= data.total} onClick={() => setOffset(offset + PAGE)}>
            Older
          </Button>
        </div>
      )}
    </div>
  );
}
